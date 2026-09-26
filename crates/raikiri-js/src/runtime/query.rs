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
//! # Known limitation: sibling combinators/structural pseudo-classes on a disconnected tree
//!
//! Every selector-matching entry point here (`querySelector`/
//! `querySelectorAll`/`matches`/`closest`) ultimately calls
//! [`raikiri_style::SelectorQuery::matches_scoped`], whose sibling
//! combinators (`+`/`~`) and structural pseudo-classes
//! (`:first-child`/`:nth-child()`/etc.) key off raikiri-dom's
//! `IS_IN_DOCUMENT` flag rather than this binding's own ancestor-chain
//! plumbing. That flag is only ever set for nodes reachable from the real
//! document root (`mark_in_document_flags`, called before every walk
//! here), so it stays clear for every node in a tree that either has never
//! been attached, or is attached only under a `DocumentFragment` (whose
//! contents are never part of the main document tree). A query against
//! such a tree can therefore under-match a sibling combinator or
//! structural pseudo-class even when both the queried element and the
//! sibling/position in question are real elements of that tree.
//! Descendant/child combinators are unaffected -- they walk the ancestor
//! chain this binding itself constructs, never that flag.

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsResult, JsValue};
use raikiri_dom::{Document, NodeKind};
use raikiri_style::{SelectorQuery, StyleDom, StyleNodeId, StyleQuirksMode};

use super::collections::{CollectionSource, html_collection, node_list};
use super::interfaces::{HTML_NS, Members, wrap_optional};
use super::node::{attribute, js_str, write_attribute};
use super::webidl::{
    dom_string, this_document, this_element, this_non_element_parent_node, this_parent_node,
    throw_dom_exception, with_state,
};

// ---- shared tree walk ---------------------------------------------------

/// `index`'s ancestor **element** chain, root side first, excluding
/// `index` itself, and stopping at the first non-Element ancestor
/// (Document, DocumentFragment, or simply a detached node with no
/// parent) rather than continuing past it.
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
    if id.is_empty() {
        return Ok(JsValue::null());
    }
    let found = with_state(context, |s| {
        let doc = s.host.document();
        find_in_tree(doc, index, |e, _| {
            (doc.element_attribute(e, "id") == Some(id.as_str())).then_some(e)
        })
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
    let found = with_state(context, |s| {
        // JS mutations leave `IS_IN_DOCUMENT` stale; the cascade matcher
        // consults it for sibling combinators and structural pseudo-classes,
        // so refresh it before walking (dirty-gated: a no-op when nothing
        // changed since the last refresh).
        s.host.document_mut().mark_in_document_flags();
        let doc = s.host.document();
        let scope = scope_for_root(doc, index);
        find_in_tree(doc, index, |e, ancestors| {
            query
                .matches_scoped(doc, StyleNodeId(e as u64), ancestors, scope)
                .then_some(e)
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
    let found = with_state(context, |s| {
        s.host.document_mut().mark_in_document_flags();
        let doc = s.host.document();
        let scope = scope_for_root(doc, index);
        descendants_matching(doc, index, |e, ancestors| {
            query.matches_scoped(doc, StyleNodeId(e as u64), ancestors, scope)
        })
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
/// ("`:scope` elements « this »" in the spec's own wording).
///
/// **Known limitation**: sibling combinators (`+`/`~`) and structural
/// pseudo-classes that key off document position consult raikiri-dom's
/// `IS_IN_DOCUMENT` bit, which is only ever set for nodes reachable from
/// the real document root; a `matches` call against an element in a
/// detached tree (never attached, or attached only under a
/// `DocumentFragment`) can therefore under-match those forms even when
/// `this` and the sibling in question are both real elements of that
/// detached tree. Descendant/child combinators are unaffected -- they walk
/// this binding's own always-accurate ancestor chain, not that bit.
fn element_matches(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let query = parsed_selector(context, args)?;
    let matched = with_state(context, |s| {
        s.host.document_mut().mark_in_document_flags();
        let doc = s.host.document();
        let ancestors = element_ancestors(doc, index);
        let scope = Some(StyleNodeId(index as u64));
        query.matches_scoped(doc, StyleNodeId(index as u64), &ancestors, scope)
    })?;
    Ok(JsValue::from(matched))
}

/// `Element.closest` (DOM §4.9): `this`'s inclusive ancestor elements, in
/// reverse tree order (`this` first, then its parent, and so on), the
/// first one that matches `selectors` with `:scope` bound to `this` --
/// fixed for every candidate in the walk, not re-derived per candidate
/// (the spec's "`:scope` elements « this »" is the same single-element set
/// throughout its own step 3 loop). See [`element_matches`]'s doc for the
/// same disconnected-tree sibling-combinator limitation.
fn element_closest(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let query = parsed_selector(context, args)?;
    let scope = Some(StyleNodeId(index as u64));
    let found = with_state(context, |s| {
        s.host.document_mut().mark_in_document_flags();
        let doc = s.host.document();
        let mut chain = element_ancestors(doc, index);
        chain.push(StyleNodeId(index as u64));
        (0..chain.len()).rev().find_map(|i| {
            let candidate = chain[i];
            query
                .matches_scoped(doc, candidate, &chain[..i], scope)
                .then_some(candidate.0 as usize)
        })
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

#[cfg(test)]
mod tests;
