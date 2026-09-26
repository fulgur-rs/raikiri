//! `Node` tree-mutation members, the `ParentNode` / `ChildNode` /
//! `NonDocumentTypeChildNode` mixins, `CharacterData`, `ProcessingInstruction`,
//! and `Document`'s node-creation methods.
//!
//! Spec refs:
//! - <https://dom.spec.whatwg.org/#interface-node>
//! - <https://dom.spec.whatwg.org/#interface-parentnode>
//! - <https://dom.spec.whatwg.org/#interface-childnode>
//! - <https://dom.spec.whatwg.org/#interface-characterdata>
//! - <https://dom.spec.whatwg.org/#interface-processinginstruction>
//! - <https://dom.spec.whatwg.org/#converting-nodes-into-a-node>

use boa_engine::{Context, JsError, JsResult, JsValue};
use raikiri_dom::{DomMutationError, NodeKind};

use super::collections::{CollectionSource, html_collection, node_list};
use super::interfaces::{Members, wrap, wrap_optional};
use super::node::{js_str, mark_dirty};
use super::webidl::{
    arg_node, arg_node_or_null, arg_required_node_or_null, dom_string, node_index,
    this_character_data, this_child_node, this_document, this_node, this_parent_node,
    this_processing_instruction, throw_dom_exception, unreachable_mutation_error, with_state,
};

/// Map a raikiri-dom mutation-validity failure to the `DOMException` it
/// corresponds to, with a fixed, generic message: `DomMutationError`'s own
/// message embeds a raikiri-dom arena index, which must never be observable
/// from script.
pub(crate) fn map_mutation_error(context: &mut Context, error: DomMutationError) -> JsError {
    match error {
        DomMutationError::HierarchyRequest(_) => throw_dom_exception(
            context,
            "HierarchyRequestError",
            "node cannot be inserted here",
        ),
        DomMutationError::NotFound(_) => throw_dom_exception(
            context,
            "NotFoundError",
            "the given node is not a child of this node",
        ),
    }
}

// ---- DOM §4.2.6 "convert nodes into a node" ----------------------------

/// A single argument of a variadic "nodes or strings" method (`append`,
/// `prepend`, `before`, `after`, `replaceWith`, `replaceChildren`): either
/// an existing `Node`, or a value to `ToString`-convert into new Text-node
/// data.
enum NodeOrText {
    Node(usize),
    Text(String),
}

/// Convert every argument, in order. `ToString` on a non-`Node` value can
/// run arbitrary script (a `toString` method), so every conversion happens
/// up front, before any of them touch the document (see [`nodes_into_a_node`]).
fn convert_args(context: &mut Context, args: &[JsValue]) -> JsResult<Vec<NodeOrText>> {
    args.iter()
        .map(|v| match node_index(v) {
            Some(index) => Ok(NodeOrText::Node(index)),
            None => Ok(NodeOrText::Text(
                v.clone().to_string(context)?.to_std_string_escaped(),
            )),
        })
        .collect()
}

/// The arena indices of the `Node`-typed items among `items` (as opposed to
/// the string-converted ones), in argument order -- the "nodes" a sibling
/// method's viable-sibling search must skip over.
fn given_node_indices(items: &[NodeOrText]) -> Vec<usize> {
    items
        .iter()
        .filter_map(|item| match item {
            NodeOrText::Node(index) => Some(*index),
            NodeOrText::Text(_) => None,
        })
        .collect()
}

/// DOM §4.2.6 "convert nodes into a node": turn the (already argument-
/// converted) list into a single node index, wrapping more than one in a
/// fresh, detached `DocumentFragment`. [`raikiri_dom::Document::pre_insert`]
/// already splices a `DocumentFragment` argument's children open in source
/// order instead of inserting it as a single child, so building one here
/// and handing it to `pre_insert` elsewhere reproduces "insert each of
/// nodes" without a second, bespoke insertion loop.
fn nodes_into_a_node(context: &mut Context, items: Vec<NodeOrText>) -> JsResult<usize> {
    let result = with_state(context, |s| -> Result<usize, DomMutationError> {
        let doc = s.host.document_mut();
        let indices: Vec<usize> = items
            .into_iter()
            .map(|item| match item {
                NodeOrText::Node(index) => index,
                NodeOrText::Text(text) => doc.create_detached_text(&text),
            })
            .collect();
        if indices.len() == 1 {
            return Ok(indices[0]);
        }
        let fragment = doc.create_detached_fragment();
        for node in indices {
            doc.pre_insert(fragment, node, None)?;
        }
        Ok(fragment)
    })?;
    result.map_err(|e| map_mutation_error(context, e))
}

/// DOM §4.2.3 "replace all with node within parent": detach every child of
/// `parent` other than `node` itself, then insert `node` (already built,
/// e.g. by [`nodes_into_a_node`]), or nothing for `None`.
///
/// `node` is excluded from the detach loop because it can already be one
/// of `parent`'s current children (e.g. `b.replaceChildren(existingChild)`)
/// -- `pre_insert`'s move (detach, then reattach at the end) leaves it
/// attached, and an unconditional detach loop over the *pre-insert*
/// snapshot of `parent`'s children would immediately detach it right back
/// out, losing it instead of keeping it. The detach loop only runs *after*
/// `pre_insert`'s own validity check, which runs against `parent`'s
/// children as they stand before this call -- exactly the "ensure
/// pre-insertion validity of node into this before null" step that precedes
/// "replace all" in every caller (`ParentNode.replaceChildren`, the
/// `textContent` setter), without a second, separately-timed check.
pub(crate) fn replace_all(
    context: &mut Context,
    parent: usize,
    node: Option<usize>,
) -> JsResult<()> {
    let result = with_state(context, |s| -> Result<(), DomMutationError> {
        let doc = s.host.document_mut();
        let previous_children = doc
            .get_node(parent)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        if let Some(node) = node {
            doc.pre_insert(parent, node, None)?;
        }
        for child in previous_children {
            if Some(child) != node {
                doc.detach_from_parent(child);
            }
        }
        Ok(())
    })?;
    result.map_err(|e| map_mutation_error(context, e))
}

// ---- Node: tree navigation and mutation --------------------------------

/// `childNodes`: one live `NodeList` per node, so that
/// `node.childNodes === node.childNodes`.
fn child_nodes(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    if let Some(existing) = with_state(context, |s| s.child_node_lists.get(&index).cloned())? {
        return Ok(existing.into());
    }
    let list = node_list(context, CollectionSource::ChildNodes(index))?;
    with_state(context, |s| s.child_node_lists.insert(index, list.clone()))?;
    Ok(list.into())
}

fn first_child(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let child = with_state(context, |s| {
        s.host
            .document()
            .get_node(index)
            .and_then(|n| n.children.first().copied())
    })?;
    wrap_optional(context, child)
}

fn last_child(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let child = with_state(context, |s| {
        s.host
            .document()
            .get_node(index)
            .and_then(|n| n.children.last().copied())
    })?;
    wrap_optional(context, child)
}

/// `node`'s previous sibling in its current parent's child order, or `None`
/// if `node` is detached or is its parent's first child.
fn previous_sibling_index(doc: &raikiri_dom::Document, node: usize) -> Option<usize> {
    let parent = doc.parent_of(node)?;
    let siblings = &doc.get_node(parent)?.children;
    let position = siblings.iter().position(|&c| c == node)?;
    let previous = position.checked_sub(1)?;
    siblings.get(previous).copied()
}

/// `node`'s next sibling in its current parent's child order, or `None` if
/// `node` is detached or is its parent's last child.
fn next_sibling_index(doc: &raikiri_dom::Document, node: usize) -> Option<usize> {
    let parent = doc.parent_of(node)?;
    let siblings = &doc.get_node(parent)?.children;
    let position = siblings.iter().position(|&c| c == node)?;
    siblings.get(position + 1).copied()
}

fn previous_sibling(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let sibling = with_state(context, |s| {
        previous_sibling_index(s.host.document(), index)
    })?;
    wrap_optional(context, sibling)
}

fn next_sibling(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let sibling = with_state(context, |s| next_sibling_index(s.host.document(), index))?;
    wrap_optional(context, sibling)
}

fn owner_document(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let owner = with_state(context, |s| {
        let doc = s.host.document();
        if doc.get_node(index).map(|n| n.kind()) == Some(NodeKind::Document) {
            None
        } else {
            Some(doc.root_index())
        }
    })?;
    wrap_optional(context, owner)
}

/// `Node.isConnected` (DOM §4.4): whether `index` is reachable from the
/// document root by walking parents, an explicit loop rather than
/// recursion (a script-built chain has no depth bound before it reaches
/// raikiri-dom).
fn is_connected(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let connected = with_state(context, |s| {
        let doc = s.host.document();
        let root = doc.root_index();
        let mut current = index;
        loop {
            if current == root {
                break true;
            }
            match doc.parent_of(current) {
                Some(parent) => current = parent,
                None => break false,
            }
        }
    })?;
    Ok(JsValue::from(connected))
}

fn has_child_nodes(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let has = with_state(context, |s| {
        s.host
            .document()
            .get_node(index)
            .is_some_and(|n| !n.children.is_empty())
    })?;
    Ok(JsValue::from(has))
}

/// `Node.nodeValue` getter (DOM §4.4): a `CharacterData` node's data;
/// `null` for every other kind.
fn node_value(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let data = with_state(context, |s| {
        match s.host.document().get_node(index).map(|n| n.kind()) {
            Some(NodeKind::Text | NodeKind::Comment | NodeKind::ProcessingInstruction) => {
                s.host.document().character_data(index).map(str::to_owned)
            }
            _ => None,
        }
    })?;
    Ok(data.map_or_else(JsValue::null, |d| js_str(&d)))
}

/// `Node.nodeValue` setter (DOM §4.4): replaces a `CharacterData` node's
/// data outright; every other kind ignores the write. `nodeValue` is typed
/// `DOMString?` (nullable): WebIDL's ES-value conversion for a nullable
/// `DOMString?` maps both `null` and `undefined` to the IDL null value, so
/// either one clears the node the same way, unlike plain `ToString`
/// (which would otherwise stringify `undefined` to `"undefined"`).
fn set_node_value(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let value = match args.first() {
        Some(v) if v.is_null() || v.is_undefined() => String::new(),
        _ => dom_string(args, 0, context)?,
    };
    let is_character_data = with_state(context, |s| {
        matches!(
            s.host.document().get_node(index).map(|n| n.kind()),
            Some(NodeKind::Text | NodeKind::Comment | NodeKind::ProcessingInstruction)
        )
    })?;
    if is_character_data {
        let result = with_state(context, |s| {
            s.host.document_mut().set_character_data(index, &value)
        })?;
        if result.is_err() {
            // cov:ignore: `is_character_data` above already confirmed a Text,
            // Comment, or ProcessingInstruction index.
            return Err(unreachable_mutation_error());
        }
        mark_dirty(context)?;
    }
    Ok(JsValue::undefined())
}

fn insert_before(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_node(this, context)?;
    let node = arg_node(args, 0, context)?;
    let before = arg_required_node_or_null(args, 1, context)?;
    let result = with_state(context, |s| {
        s.host.document_mut().pre_insert(parent, node, before)
    })?;
    result.map_err(|e| map_mutation_error(context, e))?;
    mark_dirty(context)?;
    Ok(args[0].clone())
}

fn append_child(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_node(this, context)?;
    let node = arg_node(args, 0, context)?;
    let result = with_state(context, |s| {
        s.host.document_mut().pre_insert(parent, node, None)
    })?;
    result.map_err(|e| map_mutation_error(context, e))?;
    mark_dirty(context)?;
    Ok(args[0].clone())
}

fn remove_child(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_node(this, context)?;
    let child = arg_node(args, 0, context)?;
    let result = with_state(context, |s| s.host.document_mut().pre_remove(parent, child))?;
    result.map_err(|e| map_mutation_error(context, e))?;
    mark_dirty(context)?;
    Ok(args[0].clone())
}

fn replace_child(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_node(this, context)?;
    let node = arg_node(args, 0, context)?;
    let child = arg_node(args, 1, context)?;
    let result = with_state(context, |s| {
        s.host.document_mut().replace_child(parent, node, child)
    })?;
    result.map_err(|e| map_mutation_error(context, e))?;
    mark_dirty(context)?;
    Ok(args[1].clone())
}

/// DOM §4.2.2 "contains": whether `other` is an inclusive descendant of
/// `root`, found with an explicit stack (the tree may be arbitrarily deep).
fn is_inclusive_descendant(doc: &raikiri_dom::Document, root: usize, other: usize) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node == other {
            return true;
        }
        if let Some(n) = doc.get_node(node) {
            stack.extend(n.children.iter().copied());
        }
    }
    false
}

fn contains(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_node(this, context)?;
    let other = arg_node_or_null(args, 0, context)?;
    let found = with_state(context, |s| {
        other.is_some_and(|other| is_inclusive_descendant(s.host.document(), index, other))
    })?;
    Ok(JsValue::from(found))
}

pub(crate) const NODE_TREE_MEMBERS: Members = Members {
    getters: &[
        ("childNodes", child_nodes),
        ("firstChild", first_child),
        ("lastChild", last_child),
        ("previousSibling", previous_sibling),
        ("nextSibling", next_sibling),
        ("ownerDocument", owner_document),
        ("isConnected", is_connected),
    ],
    accessors: &[("nodeValue", node_value, set_node_value)],
    methods: &[
        ("hasChildNodes", 0, has_child_nodes),
        ("insertBefore", 2, insert_before),
        ("appendChild", 1, append_child),
        ("removeChild", 1, remove_child),
        ("replaceChild", 2, replace_child),
        ("contains", 1, contains),
    ],
};

// ---- ParentNode mixin (Document, DocumentFragment, Element) -----------

/// The Element children of `parent`, in document order.
pub(crate) fn element_children_of(doc: &raikiri_dom::Document, parent: usize) -> Vec<usize> {
    doc.get_node(parent)
        .map(|n| {
            n.children
                .iter()
                .copied()
                .filter(|&c| {
                    doc.get_node(c)
                        .is_some_and(|n| n.kind() == NodeKind::Element)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `children`: one live `HTMLCollection` per node, so that
/// `node.children === node.children`.
fn children(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_parent_node(this, context)?;
    if let Some(existing) = with_state(context, |s| s.children_collections.get(&index).cloned())? {
        return Ok(existing.into());
    }
    let collection = html_collection(context, CollectionSource::Children(index))?;
    with_state(context, |s| {
        s.children_collections.insert(index, collection.clone())
    })?;
    Ok(collection.into())
}

fn first_element_child(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_parent_node(this, context)?;
    let found = with_state(context, |s| {
        element_children_of(s.host.document(), index)
            .first()
            .copied()
    })?;
    wrap_optional(context, found)
}

fn last_element_child(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_parent_node(this, context)?;
    let found = with_state(context, |s| {
        element_children_of(s.host.document(), index)
            .last()
            .copied()
    })?;
    wrap_optional(context, found)
}

fn child_element_count(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_parent_node(this, context)?;
    let count = with_state(context, |s| {
        element_children_of(s.host.document(), index).len()
    })?;
    Ok(JsValue::from(count as u32))
}

fn append(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_parent_node(this, context)?;
    let items = convert_args(context, args)?;
    let node = nodes_into_a_node(context, items)?;
    let result = with_state(context, |s| {
        s.host.document_mut().pre_insert(parent, node, None)
    })?;
    result.map_err(|e| map_mutation_error(context, e))?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

fn prepend(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_parent_node(this, context)?;
    let items = convert_args(context, args)?;
    let node = nodes_into_a_node(context, items)?;
    let before = with_state(context, |s| {
        s.host
            .document()
            .get_node(parent)
            .and_then(|n| n.children.first().copied())
    })?;
    let result = with_state(context, |s| {
        s.host.document_mut().pre_insert(parent, node, before)
    })?;
    result.map_err(|e| map_mutation_error(context, e))?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

fn replace_children(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_parent_node(this, context)?;
    let items = convert_args(context, args)?;
    let node = nodes_into_a_node(context, items)?;
    replace_all(context, parent, Some(node))?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

pub(crate) const PARENT_NODE_MEMBERS: Members = Members {
    getters: &[
        ("children", children),
        ("firstElementChild", first_element_child),
        ("lastElementChild", last_element_child),
        ("childElementCount", child_element_count),
    ],
    accessors: &[],
    methods: &[
        ("append", 0, append),
        ("prepend", 0, prepend),
        ("replaceChildren", 0, replace_children),
    ],
};

// ---- ChildNode mixin (Element, CharacterData) --------------------------

fn first_not_in(mut candidates: impl Iterator<Item = usize>, given: &[usize]) -> Option<usize> {
    candidates.find(|c| !given.contains(c))
}

/// `node`'s first preceding sibling that is not one of `given`'s indices, or
/// `None` (DOM §4.2.6's "viable previous sibling").
fn viable_previous_sibling(
    doc: &raikiri_dom::Document,
    node: usize,
    given: &[usize],
) -> Option<usize> {
    let parent = doc.parent_of(node)?;
    let siblings = &doc.get_node(parent)?.children;
    let position = siblings.iter().position(|&c| c == node)?;
    first_not_in(siblings[..position].iter().rev().copied(), given)
}

/// `node`'s first following sibling that is not one of `given`'s indices, or
/// `None` (DOM §4.2.6's "viable next sibling").
fn viable_next_sibling(doc: &raikiri_dom::Document, node: usize, given: &[usize]) -> Option<usize> {
    let parent = doc.parent_of(node)?;
    let siblings = &doc.get_node(parent)?.children;
    let position = siblings.iter().position(|&c| c == node)?;
    first_not_in(siblings[position + 1..].iter().copied(), given)
}

fn before(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let this_index = this_child_node(this, context)?;
    let items = convert_args(context, args)?;
    let given = given_node_indices(&items);
    let Some(parent) = with_state(context, |s| s.host.document().parent_of(this_index))? else {
        return Ok(JsValue::undefined());
    };
    let viable_previous = with_state(context, |s| {
        viable_previous_sibling(s.host.document(), this_index, &given)
    })?;
    let node = nodes_into_a_node(context, items)?;
    // DOM §4.2.6 `before`: the reference node is resolved *after* building
    // `node`, since doing so can itself move nodes out of `parent`.
    let reference = with_state(context, |s| {
        let doc = s.host.document();
        match viable_previous {
            Some(previous) => next_sibling_index(doc, previous),
            None => doc
                .get_node(parent)
                .and_then(|n| n.children.first().copied()),
        }
    })?;
    let result = with_state(context, |s| {
        s.host.document_mut().pre_insert(parent, node, reference)
    })?;
    result.map_err(|e| map_mutation_error(context, e))?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

fn after(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let this_index = this_child_node(this, context)?;
    let items = convert_args(context, args)?;
    let given = given_node_indices(&items);
    let Some(parent) = with_state(context, |s| s.host.document().parent_of(this_index))? else {
        return Ok(JsValue::undefined());
    };
    let viable_next = with_state(context, |s| {
        viable_next_sibling(s.host.document(), this_index, &given)
    })?;
    let node = nodes_into_a_node(context, items)?;
    let result = with_state(context, |s| {
        s.host.document_mut().pre_insert(parent, node, viable_next)
    })?;
    result.map_err(|e| map_mutation_error(context, e))?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

fn replace_with(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let this_index = this_child_node(this, context)?;
    let items = convert_args(context, args)?;
    let given = given_node_indices(&items);
    let Some(parent) = with_state(context, |s| s.host.document().parent_of(this_index))? else {
        return Ok(JsValue::undefined());
    };
    let viable_next = with_state(context, |s| {
        viable_next_sibling(s.host.document(), this_index, &given)
    })?;
    let node = nodes_into_a_node(context, items)?;
    // Building `node` can itself have moved `this` out of `parent` (e.g. if
    // `this` was one of the replacement arguments); re-check before deciding
    // how to place `node`.
    let still_child = with_state(context, |s| {
        s.host.document().parent_of(this_index) == Some(parent)
    })?;
    let result = with_state(context, |s| -> Result<(), DomMutationError> {
        if still_child {
            s.host
                .document_mut()
                .replace_child(parent, node, this_index)
        } else {
            s.host.document_mut().pre_insert(parent, node, viable_next)
        }
    })?;
    result.map_err(|e| map_mutation_error(context, e))?;
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

fn remove(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_child_node(this, context)?;
    let Some(parent) = with_state(context, |s| s.host.document().parent_of(index))? else {
        return Ok(JsValue::undefined());
    };
    let result = with_state(context, |s| s.host.document_mut().pre_remove(parent, index))?;
    if let Err(error) = result {
        // cov:ignore: `parent` was confirmed to be `index`'s current parent immediately above.
        return Err(map_mutation_error(context, error));
    }
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

pub(crate) const CHILD_NODE_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[],
    methods: &[
        ("before", 0, before),
        ("after", 0, after),
        ("replaceWith", 0, replace_with),
        ("remove", 0, remove),
    ],
};

// ---- NonDocumentTypeChildNode mixin (Element, CharacterData) ----------

fn previous_element_sibling(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = this_child_node(this, context)?;
    let found = with_state(context, |s| {
        let doc = s.host.document();
        let mut current = index;
        while let Some(previous) = previous_sibling_index(doc, current) {
            if doc
                .get_node(previous)
                .is_some_and(|n| n.kind() == NodeKind::Element)
            {
                return Some(previous);
            }
            current = previous;
        }
        None
    })?;
    wrap_optional(context, found)
}

fn next_element_sibling(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_child_node(this, context)?;
    let found = with_state(context, |s| {
        let doc = s.host.document();
        let mut current = index;
        while let Some(next) = next_sibling_index(doc, current) {
            if doc
                .get_node(next)
                .is_some_and(|n| n.kind() == NodeKind::Element)
            {
                return Some(next);
            }
            current = next;
        }
        None
    })?;
    wrap_optional(context, found)
}

pub(crate) const NON_DOCUMENT_TYPE_CHILD_NODE_MEMBERS: Members = Members {
    getters: &[
        ("previousElementSibling", previous_element_sibling),
        ("nextElementSibling", next_element_sibling),
    ],
    accessors: &[],
    methods: &[],
};

// ---- CharacterData (Text, Comment, ProcessingInstruction) --------------

fn data(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_character_data(this, context)?;
    let text = with_state(context, |s| {
        s.host
            .document()
            .character_data(index)
            .map(str::to_owned)
            .unwrap_or_default()
    })?;
    Ok(js_str(&text))
}

/// `CharacterData.data` setter (`[LegacyNullToEmptyString]`): `null` clears
/// the data instead of converting to the string `"null"`.
fn set_data(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_character_data(this, context)?;
    let value = match args.first() {
        Some(v) if v.is_null() => String::new(),
        _ => dom_string(args, 0, context)?,
    };
    let result = with_state(context, |s| {
        s.host.document_mut().set_character_data(index, &value)
    })?;
    if result.is_err() {
        // cov:ignore: `this_character_data` above already confirmed a Text,
        // Comment, or ProcessingInstruction index.
        return Err(unreachable_mutation_error());
    }
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

/// `CharacterData.length`: the data's length in UTF-16 code units (DOM
/// §4.10), not raikiri-dom's own UTF-8 byte count.
fn length(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_character_data(this, context)?;
    let len = with_state(context, |s| {
        s.host
            .document()
            .character_data(index)
            .map(|d| d.encode_utf16().count())
            .unwrap_or(0)
    })?;
    Ok(JsValue::from(len as u32))
}

fn append_data(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_character_data(this, context)?;
    let addition = dom_string(args, 0, context)?;
    let current = with_state(context, |s| {
        s.host
            .document()
            .character_data(index)
            .map(str::to_owned)
            .unwrap_or_default()
    })?;
    let updated = current + &addition;
    let result = with_state(context, |s| {
        s.host.document_mut().set_character_data(index, &updated)
    })?;
    if result.is_err() {
        // cov:ignore: `this_character_data` above already confirmed a Text,
        // Comment, or ProcessingInstruction index.
        return Err(unreachable_mutation_error());
    }
    mark_dirty(context)?;
    Ok(JsValue::undefined())
}

pub(crate) const CHARACTER_DATA_MEMBERS: Members = Members {
    getters: &[("length", length)],
    accessors: &[("data", data, set_data)],
    methods: &[("appendData", 1, append_data)],
};

// ---- ProcessingInstruction ----------------------------------------------

fn target(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_processing_instruction(this, context)?;
    let target = with_state(context, |s| {
        s.host
            .document()
            .processing_instruction_target(index)
            .map(str::to_owned)
            .unwrap_or_default()
    })?;
    Ok(js_str(&target))
}

pub(crate) const PROCESSING_INSTRUCTION_MEMBERS: Members = Members {
    getters: &[("target", target)],
    accessors: &[],
    methods: &[],
};

// ---- Document node-creation methods -------------------------------------

fn create_text_node(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    this_document(this, context)?;
    let data = dom_string(args, 0, context)?;
    let index = with_state(context, |s| {
        s.host.document_mut().create_detached_text(&data)
    })?;
    mark_dirty(context)?;
    Ok(wrap(context, index)?.into())
}

fn create_comment(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    this_document(this, context)?;
    let data = dom_string(args, 0, context)?;
    let index = with_state(context, |s| {
        s.host.document_mut().create_detached_comment(&data)
    })?;
    mark_dirty(context)?;
    Ok(wrap(context, index)?.into())
}

fn create_document_fragment(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    this_document(this, context)?;
    let index = with_state(context, |s| {
        s.host.document_mut().create_detached_fragment()
    })?;
    mark_dirty(context)?;
    Ok(wrap(context, index)?.into())
}

pub(crate) const DOCUMENT_CREATE_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[],
    methods: &[
        ("createTextNode", 1, create_text_node),
        ("createComment", 1, create_comment),
        ("createDocumentFragment", 0, create_document_fragment),
    ],
};

#[cfg(test)]
mod tests;
