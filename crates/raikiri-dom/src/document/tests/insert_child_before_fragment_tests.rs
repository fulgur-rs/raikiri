//! Check that `insert_child_before` accepts
//! [`NodeData::DocumentFragment`] as a child and splices the fragment’s
//! children into parent.children at `before` in source order.
//! This pins fragment-aware semantics (WHATWG DOM §4.2.3 Mutation
//! algorithms — insert algorithm steps 1 + 4.1 + 7.3).
//! It is the positional counterpart of attach_child (tail append), resolving
//! a previously latent asymmetry.
//!
//! Spec ref:
//! - <https://dom.spec.whatwg.org/#concept-node-pre-insert>
//! - <https://dom.spec.whatwg.org/#concept-node-insert>
//!
//! Contract (five tests mirroring `attach_child_fragment_tests`):
//! (a) splice the fragment’s children into parent.children at `before`
//! in source order
//! (b) empty the fragment’s children Vec (move, not clone)
//! (c) exclude the fragment node itself from parent.children
//! (d) an empty-fragment splice leaves parent.children unchanged (edge case)
//! (e) non-fragment children retain the old insertion behavior (regression check)
use super::*;
use crate::node::NodeData;

fn make_fragment_with_two_children(doc: &mut Document) -> (usize, usize, usize) {
    let frag = doc.nodes.len();
    doc.nodes.push(Node::new_document_fragment());
    // The fragment’s children are two detached Elements.
    let c0 = doc.append_element(Some(frag), "span", Style::default(), None::<&str>);
    let c1 = doc.append_element(Some(frag), "div", Style::default(), None::<&str>);
    (frag, c0, c1)
}

#[test]
fn insert_child_before_splices_fragment_children_at_position() {
    // (a): insert the fragment’s children into parent.children at `before`
    // in source order. Unlike attach_child’s tail append, test the positional
    // splice: all fragment children immediately precede `before`, followed by
    // `before` itself and any later existing siblings.
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    // The parent already has two siblings: `[first, last]`.
    let first = doc.append_element(Some(parent), "h1", Style::default(), None::<&str>);
    let last = doc.append_element(Some(parent), "footer", Style::default(), None::<&str>);

    let (frag, c0, c1) = make_fragment_with_two_children(&mut doc);
    doc.insert_child_before(parent, last, frag);

    // Insert fragment children c0, c1 before last (between first and last).
    assert_eq!(
        doc.nodes[parent].children,
        vec![first, c0, c1, last],
        "insert_child_before with DocumentFragment must splice fragment's children at the `before` position in source order"
    );
}

#[test]
fn insert_child_before_empties_fragments_children_after_move() {
    // (b): empty the fragment’s children Vec (move semantics, not clone).
    // The old behavior (simply inserting the fragment itself) would retain its
    // children; this test checks that draining moves them.
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let sibling = doc.append_element(Some(parent), "p", Style::default(), None::<&str>);

    let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);
    // Precondition: the fragment itself has two children.
    assert_eq!(doc.nodes[frag].children.len(), 2);

    doc.insert_child_before(parent, sibling, frag);
    assert!(
        doc.nodes[frag].children.is_empty(),
        "fragment's children must be drained after insert_child_before (move semantics per WHATWG DOM §4.2.3 Mutation algorithms — insert step 4.1)"
    );
    // The fragment itself remains in the arena (kind = DocumentFragment, detached).
    assert!(matches!(doc.nodes[frag].data, NodeData::DocumentFragment));
}

#[test]
fn insert_child_before_does_not_insert_the_fragment_node_itself() {
    // (c): the fragment node itself must never appear in parent.children.
    // WHATWG DOM §4.2.3 Mutation algorithms — insert step 1 defines nodes
    // as fragment.children for a fragment, excluding the fragment itself from
    // tree insertion (mutation records likewise observe parent → fragment’s
    // children rather than parent → fragment).
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let sibling = doc.append_element(Some(parent), "p", Style::default(), None::<&str>);

    let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);
    doc.insert_child_before(parent, sibling, frag);

    assert!(
        !doc.nodes[parent].children.contains(&frag),
        "fragment node itself must NOT appear in parent.children (spec: fragment is unrendered container)"
    );
}

#[test]
fn insert_child_before_with_empty_fragment_is_noop_on_parent_children() {
    // (d): splicing an empty fragment does not change the parent’s children.
    // Check that splice(pos..pos, empty_vec) does nothing (the positional
    // counterpart of the attach_child edge check).
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let a = doc.append_element(Some(parent), "a", Style::default(), None::<&str>);
    let b = doc.append_element(Some(parent), "b", Style::default(), None::<&str>);

    // Empty fragment.
    let empty_frag = doc.nodes.len();
    doc.nodes.push(Node::new_document_fragment());

    doc.insert_child_before(parent, b, empty_frag);
    assert_eq!(
        doc.nodes[parent].children,
        vec![a, b],
        "empty fragment insert_child_before must not change parent.children"
    );
    assert!(!doc.nodes[parent].children.contains(&empty_frag));
}

#[test]
fn insert_child_before_non_fragment_keeps_existing_insert_semantics() {
    // (e) (acceptance 4): non-fragment children (Element / Text / Comment
    // / PI / Document) retain the old behavior (simple insert at `before`);
    // pin that the fragment branch causes no collateral damage.
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let last = doc.append_element(Some(parent), "z", Style::default(), None::<&str>);
    // Insert a detached Element at before=last (a foster-parenting primitive).
    let elem = doc.append_element(None, "m", Style::default(), None::<&str>);
    doc.insert_child_before(parent, last, elem);
    assert_eq!(doc.nodes[parent].children, vec![elem, last]);
}
