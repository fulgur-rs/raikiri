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

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsResult, JsValue};
use raikiri_dom::{Document, NodeKind};
use raikiri_style::{SelectorQuery, StyleNodeId};

use super::interfaces::{Members, wrap, wrap_optional};
use super::node::{attribute, js_str, write_attribute};
use super::webidl::{
    dom_string, this_document, this_element, this_non_element_parent_node, this_parent_node,
    throw_dom_exception, with_state,
};

/// The HTML namespace URI, for DOM §4.4 "HTML namespace + HTML document"
/// tag-name matching. Duplicated from `interfaces.rs`'s own private
/// constant of the same value rather than shared, following this runtime's
/// existing convention of a local literal per module (`node.rs`'s
/// `html_uppercased_name` does the same).
const HTML_NS: &str = "http://www.w3.org/1999/xhtml";

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
/// scoping root). `visit` is called with that element's real ancestor
/// **element** chain, root side first -- seeded from `root`'s own real
/// ancestors (and `root` itself, when `root` is an Element) rather than
/// starting empty at `root`, so a combinator selector still sees the whole
/// document tree, not just the subtree below `root`; only which elements
/// count as *candidates* is scoped, matching how DOM §4.2.6 "match a
/// selector against a tree" resolves `:scope` against a scoping root
/// without restricting ordinary combinator matching to its subtree. This
/// is the ancestor shape [`raikiri_style::SelectorQuery::matches`] expects,
/// and what `:root` (`ancestors.is_empty()`) relies on.
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
/// order. The collections this runtime returns today are plain, static
/// arrays snapshotted at call time; live collection objects that stay in
/// sync with later mutations are not implemented.
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

/// A static Array of node wrappers, in the given tree order. Named
/// distinctly from [`element_collection`] (both build the same kind of
/// value today) so that a later, live collection type can replace either
/// one independently at this single call site.
fn static_node_list(context: &mut Context, indices: Vec<usize>) -> JsResult<JsValue> {
    let mut items = Vec::with_capacity(indices.len());
    for index in indices {
        items.push(wrap(context, index)?.into());
    }
    Ok(JsArray::from_iter(items, context).into())
}

/// A static Array of node wrappers for a `getElementsBy*` result. See
/// [`static_node_list`].
fn element_collection(context: &mut Context, indices: Vec<usize>) -> JsResult<JsValue> {
    static_node_list(context, indices)
}

/// A parsed selector list, or a thrown `SyntaxError` `DOMException` (DOM
/// §4.2.6, every selector-consuming method: `querySelector`,
/// `querySelectorAll`, `matches`, `closest`).
fn parsed_selector(context: &mut Context, args: &[JsValue]) -> JsResult<SelectorQuery> {
    let source = dom_string(args, 0, context)?;
    SelectorQuery::parse(&source)
        .map_err(|message| throw_dom_exception(context, "SyntaxError", &message))
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
        find_in_tree(doc, index, |e, ancestors| {
            query
                .matches(doc, StyleNodeId(e as u64), ancestors)
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
        descendants_matching(doc, index, |e, ancestors| {
            query.matches(doc, StyleNodeId(e as u64), ancestors)
        })
    })?;
    static_node_list(context, found)
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

/// DOM §4.4 "HTML namespace + HTML document" tag-name matching: an
/// HTML-namespace element's qualified name compares to `query`
/// ASCII-case-insensitively; any other namespace compares case-sensitively.
fn tag_name_matches(doc: &Document, node: usize, query: &str) -> bool {
    doc.get_node(node)
        .and_then(|n| n.tag_name())
        .is_some_and(|tag| {
            if doc.element_namespace_uri(node) == Some(HTML_NS) {
                tag.eq_ignore_ascii_case(query)
            } else {
                tag == query
            }
        })
}

fn elements_by_tag_name(doc: &Document, root: usize, query: &str) -> Vec<usize> {
    if query == "*" {
        descendants_matching(doc, root, |_, _| true)
    } else {
        descendants_matching(doc, root, |node, _| tag_name_matches(doc, node, query))
    }
}

/// DOM §4.4 `getElementsByClassName`: an empty token list (the empty
/// string, or a string containing only ASCII whitespace) matches nothing.
fn class_name_matches(doc: &Document, node: usize, tokens: &[String]) -> bool {
    let class_attr = attribute_str(doc, node, "class");
    tokens
        .iter()
        .all(|t| class_attr.split_ascii_whitespace().any(|c| c == t))
}

fn attribute_str<'a>(doc: &'a Document, node: usize, name: &str) -> &'a str {
    doc.element_attribute(node, name).unwrap_or("")
}

fn elements_by_class_name(doc: &Document, root: usize, query: &str) -> Vec<usize> {
    let tokens: Vec<String> = query.split_ascii_whitespace().map(str::to_owned).collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    descendants_matching(doc, root, |node, _| class_name_matches(doc, node, &tokens))
}

fn get_elements_by_tag_name_from(
    context: &mut Context,
    index: usize,
    args: &[JsValue],
) -> JsResult<JsValue> {
    let query = dom_string(args, 0, context)?;
    let found = with_state(context, |s| {
        elements_by_tag_name(s.host.document(), index, &query)
    })?;
    element_collection(context, found)
}

fn get_elements_by_class_name_from(
    context: &mut Context,
    index: usize,
    args: &[JsValue],
) -> JsResult<JsValue> {
    let query = dom_string(args, 0, context)?;
    let found = with_state(context, |s| {
        elements_by_class_name(s.host.document(), index, &query)
    })?;
    element_collection(context, found)
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

fn element_matches(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let query = parsed_selector(context, args)?;
    let matched = with_state(context, |s| {
        s.host.document_mut().mark_in_document_flags();
        let doc = s.host.document();
        let ancestors = element_ancestors(doc, index);
        query.matches(doc, StyleNodeId(index as u64), &ancestors)
    })?;
    Ok(JsValue::from(matched))
}

/// `Element.closest` (DOM §4.2.6): `this`'s inclusive ancestor elements, in
/// reverse tree order (`this` first, then its parent, and so on), the
/// first one that matches `selectors`.
fn element_closest(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let query = parsed_selector(context, args)?;
    let found = with_state(context, |s| {
        s.host.document_mut().mark_in_document_flags();
        let doc = s.host.document();
        let mut chain = element_ancestors(doc, index);
        chain.push(StyleNodeId(index as u64));
        (0..chain.len()).rev().find_map(|i| {
            let candidate = chain[i];
            query
                .matches(doc, candidate, &chain[..i])
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
