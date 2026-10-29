//! Element-scoped and document-scoped selector queries: the
//! `NonElementParentNode` mixin (`getElementById`), the `ParentNode`
//! mixin's `querySelector` / `querySelectorAll`, and `Element`'s own
//! `matches`, `closest`, `getElementsByTagName`, `getElementsByClassName`,
//! `getAttributeNames`, `id`, and `className`.
//!
//! Spec refs:
//! - <https://dom.spec.whatwg.org/#interface-nonelementparentnode>
//! - <https://dom.spec.whatwg.org/#interface-parentnode>
//! - <https://dom.spec.whatwg.org/#interface-element>
//!
//! Sibling combinators (`+`/`~`) and structural pseudo-classes
//! (`:first-child`/`:nth-child()`/etc.) work on disconnected trees as well as
//! attached ones: [`raikiri_style::SelectorQuery::matches_scoped`] gates those
//! candidates by same-tree-as-subject (the subject's own parent via
//! `parent_id`), not by raikiri-dom's `IS_IN_DOCUMENT` flag. `:root` still
//! matches only the document element (parent is the `Document` node and
//! in-document), never a disconnected root or `DocumentFragment` top-level
//! child.

mod id_cache;
pub(super) use id_cache::IdLookupCache;

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsResult, JsValue};
use raikiri_dom::{Document, NodeKind};
use raikiri_style::{
    CascadeError, SelectorQuery, SelectorWorkBudget, StyleDom, StyleNodeId, StyleQuirksMode,
};

use super::collections::{CollectionSource, html_collection, node_list};
use super::interfaces::{HTML_NS, Members, wrap_optional};
use super::node::{attribute, js_str, write_attribute};
use super::webidl::{
    dom_string, this_document, this_element, this_non_element_parent_node, this_parent_node,
    throw_dom_exception, with_state,
};

// ---- shared tree walk ---------------------------------------------------

/// Real ancestor elements, nearest parent last. Parent pointers are O(1),
/// including in detached and template fragment trees.
fn element_ancestors(doc: &Document, index: usize) -> Vec<StyleNodeId> {
    let mut chain = Vec::new();
    let mut current = index;
    while let Some(parent) = doc.parent_of(current) {
        if doc.get_node(parent).map(|n| n.kind()) != Some(NodeKind::Element) {
            break;
        }
        chain.push(StyleNodeId(parent as u64));
        current = parent;
    }
    chain.reverse();
    chain
}

fn bounded_ancestors(
    doc: &Document,
    index: usize,
    budget: &SelectorWorkBudget,
) -> Result<Vec<StyleNodeId>, CascadeError> {
    budget.charge(1)?;
    let mut chain = Vec::new();
    let mut current = index;
    while let Some(parent) = doc.parent_of(current) {
        budget.charge(1)?;
        if doc.get_node(parent).map(|n| n.kind()) != Some(NodeKind::Element) {
            break;
        }
        chain.push(StyleNodeId(parent as u64));
        current = parent;
    }
    chain.reverse();
    Ok(chain)
}

/// One native selector boundary. Flag refresh is dirty-gated semantic DOM
/// maintenance, outside the logical query work count; it can scan the arena
/// once after mutation. Selector parsing and string bytes are also excluded.
/// Charge the invocation first so zero or terminal budgets touch no DOM.
fn with_selector_query<T>(
    context: &mut Context,
    run: impl FnOnce(&Document, &SelectorWorkBudget) -> Result<T, CascadeError>,
) -> JsResult<T> {
    let result = with_state(context, |s| {
        s.event_loop.selector_work.charge(1)?;
        s.host.document_mut().mark_in_document_flags();
        run(s.host.document(), &s.event_loop.selector_work)
    })?;
    result.map_err(|_| {
        let reason = super::event_loop::abort(context, super::event_loop::Abort::SelectorWork);
        super::event_loop::abort_error(reason)
    })
}

/// Fallible pre-order traversal. Every root read, child enqueue and pending
/// node visit is charged before inspection or allocation. Ancestors follow
/// parent pointers rather than allocating or scanning a whole-arena map.
fn find_in_tree_bounded<T>(
    doc: &Document,
    root: usize,
    budget: &SelectorWorkBudget,
    mut visit: impl FnMut(usize, &[StyleNodeId]) -> Result<Option<T>, CascadeError>,
) -> Result<Option<T>, CascadeError> {
    budget.charge(1)?;
    let Some(root_node) = doc.get_node(root) else {
        return Ok(None); // cov:ignore: query entrypoints validate the ParentNode handle with kind_of before borrowing this immutable Document
    };
    let mut ancestors = bounded_ancestors(doc, root, budget)?;
    if root_node.kind() == NodeKind::Element {
        ancestors.push(StyleNodeId(root as u64));
    }
    let base_depth = ancestors.len();
    let mut stack = Vec::new();
    for &child in root_node.children.iter().rev() {
        budget.charge(1)?;
        stack.push((child, base_depth));
    }
    while let Some((node, depth)) = stack.pop() {
        budget.charge(1)?;
        ancestors.truncate(depth);
        let Some(n) = doc.get_node(node) else {
            continue; // cov:ignore: pending ids come from children of resolved nodes in this immutably borrowed arena
        };
        let child_depth = if n.kind() == NodeKind::Element {
            if let Some(found) = visit(node, &ancestors)? {
                return Ok(Some(found));
            }
            ancestors.push(StyleNodeId(node as u64));
            depth + 1
        } else {
            depth
        };
        for &child in n.children.iter().rev() {
            budget.charge(1)?;
            stack.push((child, child_depth));
        }
    }
    Ok(None)
}

/// Pre-order walk of every **element** that is a descendant of `root` in
/// the `ParentNode` mixin's sense: `root` itself is never visited, even
/// when it is an Element (a scoped `querySelector` never matches its own
/// scoping root -- that is a separate question from whether `:scope`
/// matches `root`, which [`scope_for_root`]/`matches_scoped` handle).
/// `visit` is called with that element's real ancestor **element** chain,
/// root side first -- seeded from `root`'s own real ancestors (and `root`
/// itself, when `root` is an Element) rather than starting empty at
/// `root`, so an ordinary combinator selector (e.g. `body p`) still sees
/// the whole document tree, not just the subtree below `root`; only which
/// elements count as *candidates* is scoped. This is the ancestor shape
/// [`raikiri_style::SelectorQuery::matches`]/`matches_scoped` expects, and
/// what `:root` (`ancestors.is_empty()`) relies on.
///
/// Stops at the first `Some`. Iterative, with an explicit `(node, ancestor
/// depth)` stack rather than the native call stack: a script can build an
/// arbitrarily deep tree (there is no other bound on tree depth before it
/// reaches raikiri-dom), and a stack overflow there would abort the
/// process rather than raise a catchable error. `ancestors` is truncated
/// to each entry's recorded depth before that entry runs, the same
/// "truncate on pop" shape `raikiri_style::cascade::selector_match`'s own
/// explicit-stack walks use for the same reason.
fn find_in_tree<T>(
    doc: &Document,
    root: usize,
    mut visit: impl FnMut(usize, &[StyleNodeId]) -> Option<T>,
) -> Option<T> {
    let root_node = doc.get_node(root)?;
    let mut ancestors = element_ancestors(doc, root);
    if root_node.kind() == NodeKind::Element {
        ancestors.push(StyleNodeId(root as u64));
    }
    let base_depth = ancestors.len();
    let mut stack: Vec<(usize, usize)> = root_node
        .children
        .iter()
        .rev()
        .map(|&c| (c, base_depth))
        .collect();
    while let Some((node, depth)) = stack.pop() {
        ancestors.truncate(depth);
        let Some(n) = doc.get_node(node) else {
            continue; // cov:ignore: every stack entry comes from a resolved node's own children, always valid in the same arena
        };
        let child_depth = if n.kind() == NodeKind::Element {
            if let Some(found) = visit(node, &ancestors) {
                return Some(found);
            }
            ancestors.push(StyleNodeId(node as u64));
            depth + 1
        } else {
            depth
        };
        // Push in reverse so the LIFO stack pops children back in document order.
        stack.extend(n.children.iter().rev().map(|&c| (c, child_depth)));
    }
    None
}

/// Every descendant element of `root` (in the same "never `root` itself"
/// sense as [`find_in_tree`]) for which `pred` returns `true`, in tree
/// order. Live collections call this again on every access.
pub(crate) fn descendants_matching(
    doc: &Document,
    root: usize,
    mut pred: impl FnMut(usize, &[StyleNodeId]) -> bool,
) -> Vec<usize> {
    let mut out = Vec::new();
    find_in_tree::<()>(doc, root, |node, ancestors| {
        if pred(node, ancestors) {
            out.push(node);
        }
        None
    });
    out
}

/// A parsed selector list, or a thrown `SyntaxError` `DOMException` (DOM
/// §4.2.6 `querySelector`/`querySelectorAll`; DOM §4.9 `Element.matches`/
/// `closest`).
fn parsed_selector(context: &mut Context, args: &[JsValue]) -> JsResult<SelectorQuery> {
    let source = dom_string(args, 0, context)?;
    SelectorQuery::parse(&source)
        .map_err(|message| throw_dom_exception(context, "SyntaxError", &message))
}

/// The `:scope` element (CSS Selectors L4 §14.3.3) for a `querySelector`/
/// `querySelectorAll` call rooted at `root`: `root` itself when it is an
/// Element -- an `Element`-scoped call binds its own scoping root as
/// `:scope` (DOM §4.2.6) -- or `None` for a Document/DocumentFragment
/// root, which has no element of its own to bind; `:scope` then falls back
/// to `:root` semantics (see [`SelectorQuery::matches_scoped`]'s doc).
fn scope_for_root(doc: &Document, root: usize) -> Option<StyleNodeId> {
    (doc.get_node(root)?.kind() == NodeKind::Element).then_some(StyleNodeId(root as u64))
}

// ---- NonElementParentNode mixin (Document, DocumentFragment) ------------

fn get_element_by_id(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_non_element_parent_node(this, context)?;
    let id = dom_string(args, 0, context)?;
    let found = with_state(context, |s| {
        s.id_lookups.find(s.host.document(), index, &id)
    })?;
    wrap_optional(context, found)
}

pub(crate) const NON_ELEMENT_PARENT_NODE_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[],
    methods: &[("getElementById", 1, get_element_by_id)],
};

// ---- ParentNode mixin: querySelector / querySelectorAll -----------------
// (Document, DocumentFragment, Element)

fn query_selector(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_parent_node(this, context)?;
    let query = parsed_selector(context, args)?;
    let found = with_selector_query(context, |doc, budget| {
        let matcher = query.bounded_matcher(doc, scope_for_root(doc, index), budget);
        find_in_tree_bounded(doc, index, budget, |e, ancestors| {
            Ok(matcher
                .try_matches(StyleNodeId(e as u64), ancestors)?
                .then_some(e))
        })
    })?;
    wrap_optional(context, found)
}

fn query_selector_all(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_parent_node(this, context)?;
    let query = parsed_selector(context, args)?;
    let found = with_selector_query(context, |doc, budget| {
        let matcher = query.bounded_matcher(doc, scope_for_root(doc, index), budget);
        let mut found = Vec::new();
        find_in_tree_bounded::<()>(doc, index, budget, |e, ancestors| {
            if matcher.try_matches(StyleNodeId(e as u64), ancestors)? {
                budget.charge(1)?;
                found.push(e);
            }
            Ok(None)
        })?;
        Ok(found)
    })?;
    Ok(node_list(context, CollectionSource::Static(found))?.into())
}

pub(crate) const PARENT_NODE_QUERY_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[],
    methods: &[
        ("querySelector", 1, query_selector),
        ("querySelectorAll", 1, query_selector_all),
    ],
};

// ---- getElementsByTagName / getElementsByClassName (Document, Element) -

/// DOM §4.4 "HTML namespace + HTML document" tag-name matching: `query`
/// is ASCII-lowercased once up front (the spec's own "let qualifiedName be
/// qualifiedName, converted to ASCII lowercase" step, done here rather
/// than per candidate), then compared exactly against an HTML-namespace
/// element's own qualified name -- which is already lowercase, since
/// every element this runtime can produce is either script-created
/// (`Document.createElement` itself lowercases, see `node.rs::
/// create_element`) or came from an HTML parser (which lowercases tag
/// names during tokenization). Any other namespace compares `query`
/// verbatim, case-sensitively.
fn tag_name_matches(doc: &Document, node: usize, query: &str, query_lower: &str) -> bool {
    doc.get_node(node)
        .and_then(|n| n.tag_name())
        .is_some_and(|tag| {
            if doc.element_namespace_uri(node) == Some(HTML_NS) {
                tag == query_lower
            } else {
                tag == query
            }
        })
}

pub(crate) fn elements_by_tag_name(doc: &Document, root: usize, query: &str) -> Vec<usize> {
    if query == "*" {
        return descendants_matching(doc, root, |_, _| true);
    }
    let query_lower = query.to_ascii_lowercase();
    descendants_matching(doc, root, |node, _| {
        tag_name_matches(doc, node, query, &query_lower)
    })
}

/// DOM §4.4 `getElementsByClassName`: an empty token list (the empty
/// string, or a string containing only ASCII whitespace) matches nothing.
/// `quirks_html` (CSS Selectors L4's class-selector quirks-mode rule,
/// applied here to `getElementsByClassName`'s own token-set matching
/// rather than a `.foo` selector) ASCII-case-folds every token comparison
/// under quirks mode, matching `raikiri_style::cascade::selector_match`'s
/// `Component::Class` arm.
fn class_name_matches(doc: &Document, node: usize, tokens: &[String], quirks: bool) -> bool {
    let class_attr = attribute_str(doc, node, "class");
    let class_tokens: Vec<&str> = class_attr.split_ascii_whitespace().collect();
    tokens.iter().all(|t| {
        if quirks {
            class_tokens.iter().any(|c| c.eq_ignore_ascii_case(t))
        } else {
            class_tokens.contains(&t.as_str())
        }
    })
}

fn attribute_str<'a>(doc: &'a Document, node: usize, name: &str) -> &'a str {
    doc.element_attribute(node, name).unwrap_or("")
}

/// The class tokens of a `getElementsByClassName` argument.
fn class_tokens(query: &str) -> Vec<String> {
    query.split_ascii_whitespace().map(str::to_owned).collect()
}

/// Descendant elements of `root` carrying every one of `tokens`; an empty
/// token list matches nothing.
pub(crate) fn elements_with_class_tokens(
    doc: &Document,
    root: usize,
    tokens: &[String],
) -> Vec<usize> {
    if tokens.is_empty() {
        return Vec::new();
    }
    // `Document` has its own inherent `quirks_mode()` (raikiri_traits::
    // QuirksMode, the parser-facing 3-way value) as well as this
    // `StyleDom::quirks_mode()` (raikiri_style::StyleQuirksMode, the one
    // CSS selector matching itself consults) -- the inherent method shadows
    // the trait one under plain method-call syntax, so the trait method is
    // named explicitly here to reach the value this comparison needs.
    let quirks = StyleDom::quirks_mode(doc) == StyleQuirksMode::Quirks;
    descendants_matching(doc, root, |node, _| {
        class_name_matches(doc, node, tokens, quirks)
    })
}

fn get_elements_by_tag_name_from(
    context: &mut Context,
    index: usize,
    args: &[JsValue],
) -> JsResult<JsValue> {
    let query = dom_string(args, 0, context)?;
    let collection = html_collection(context, CollectionSource::TagName(index, query))?;
    Ok(collection.into())
}

fn get_elements_by_class_name_from(
    context: &mut Context,
    index: usize,
    args: &[JsValue],
) -> JsResult<JsValue> {
    let tokens = class_tokens(&dom_string(args, 0, context)?);
    let collection = html_collection(context, CollectionSource::ClassNames(index, tokens))?;
    Ok(collection.into())
}

fn document_get_elements_by_tag_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_document(this, context)?;
    get_elements_by_tag_name_from(context, index, args)
}

fn document_get_elements_by_class_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_document(this, context)?;
    get_elements_by_class_name_from(context, index, args)
}

pub(crate) const DOCUMENT_QUERY_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[],
    methods: &[
        ("getElementsByTagName", 1, document_get_elements_by_tag_name),
        (
            "getElementsByClassName",
            1,
            document_get_elements_by_class_name,
        ),
    ],
};

// ---- Element: matches, closest, getElementsBy*, getAttributeNames, -----
// ---- id, className -------------------------------------------------------

/// `Element.matches` (DOM §4.9): binds `:scope` to `this` itself
/// ("`:scope` elements « this »" in the spec's own wording). Sibling
/// combinators and structural pseudo-classes use same-tree gating, so they
/// match correctly on disconnected trees as well.
fn element_matches(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let query = parsed_selector(context, args)?;
    let matched = with_selector_query(context, |doc, budget| {
        let ancestors = bounded_ancestors(doc, index, budget)?;
        let scope = Some(StyleNodeId(index as u64));
        query.try_matches_scoped(doc, StyleNodeId(index as u64), &ancestors, scope, budget)
    })?;
    Ok(JsValue::from(matched))
}

/// `Element.closest` (DOM §4.9): `this`'s inclusive ancestor elements, in
/// reverse tree order (`this` first, then its parent, and so on), the
/// first one that matches `selectors` with `:scope` bound to `this` --
/// fixed for every candidate in the walk, not re-derived per candidate
/// (the spec's "`:scope` elements « this »" is the same single-element set
/// throughout its own step 3 loop). Sibling and structural matching use the
/// same same-tree gating as [`element_matches`].
fn element_closest(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let query = parsed_selector(context, args)?;
    let scope = Some(StyleNodeId(index as u64));
    let found = with_selector_query(context, |doc, budget| {
        let mut chain = bounded_ancestors(doc, index, budget)?;
        budget.charge(1)?;
        chain.push(StyleNodeId(index as u64));
        let matcher = query.bounded_matcher(doc, scope, budget);
        for i in (0..chain.len()).rev() {
            budget.charge(1)?;
            let candidate = chain[i];
            if matcher.try_matches(candidate, &chain[..i])? {
                return Ok(Some(candidate.0 as usize));
            }
        }
        Ok(None)
    })?;
    wrap_optional(context, found)
}

fn element_get_elements_by_tag_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    get_elements_by_tag_name_from(context, index, args)
}

fn element_get_elements_by_class_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    get_elements_by_class_name_from(context, index, args)
}

fn get_attribute_names(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let names = with_state(context, |s| {
        s.host.document().element_attribute_names(index)
    })?;
    let items: Vec<JsValue> = names.iter().map(|n| js_str(n)).collect();
    Ok(JsArray::from_iter(items, context).into())
}

fn id(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    Ok(js_str(
        &attribute(context, index, "id")?.unwrap_or_default(),
    ))
}

fn set_id(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let value = dom_string(args, 0, context)?;
    write_attribute(context, index, "id", &value)?;
    Ok(JsValue::undefined())
}

fn class_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    Ok(js_str(
        &attribute(context, index, "class")?.unwrap_or_default(),
    ))
}

fn set_class_name(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let value = dom_string(args, 0, context)?;
    write_attribute(context, index, "class", &value)?;
    Ok(JsValue::undefined())
}

fn lang(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    Ok(js_str(
        &attribute(context, index, "lang")?.unwrap_or_default(),
    ))
}

fn set_lang(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let value = dom_string(args, 0, context)?;
    write_attribute(context, index, "lang", &value)?;
    Ok(JsValue::undefined())
}

pub(crate) const ELEMENT_QUERY_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[
        ("id", id, set_id),
        ("className", class_name, set_class_name),
    ],
    methods: &[
        ("matches", 1, element_matches),
        ("closest", 1, element_closest),
        ("getElementsByTagName", 1, element_get_elements_by_tag_name),
        (
            "getElementsByClassName",
            1,
            element_get_elements_by_class_name,
        ),
        ("getAttributeNames", 0, get_attribute_names),
    ],
};

pub(crate) const HTML_ELEMENT_QUERY_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[("lang", lang, set_lang)],
    methods: &[],
};

#[cfg(test)]
mod tests;
