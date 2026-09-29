//! Check the contract when attach_child receives
//! `NodeData::DocumentFragment` as a child: it must implement the
//! fragment-aware semantics of WHATWG DOM §4.2.3 Mutation algorithms —
//! insert algorithm steps 1 + 4.1 + 7.2 (append corresponds to step 7.2,
//! not the positional splice in step 7.3; see the documentation for
//! `Document::attach_child`).
//!
//! Spec ref:
//! - <https://dom.spec.whatwg.org/#concept-node-pre-insert>
//! - <https://dom.spec.whatwg.org/#concept-node-insert>
//!
//! Contract (split into five tests):
//! (a) extend parent.children with fragment children in source order
//! (b) empty the fragment’s children Vec (move, not clone)
//! (c) exclude the fragment node itself from parent.children
//! (d) attaching an empty fragment leaves parent.children unchanged (edge case)
//! (e) non-fragment children retain the old push behavior (regression check)
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
fn attach_child_extends_parent_with_fragment_children_in_order() {
    // WHATWG DOM §4.2.3 Mutation algorithms — insert steps 1 + 7.2 with a
    // fragment child: node's children → parent's children, in tree order.
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    // Start with one existing child of the parent.
    let pre_existing = doc.append_element(Some(parent), "pre", Style::default(), None::<&str>);

    let (frag, c0, c1) = make_fragment_with_two_children(&mut doc);
    doc.attach_child(parent, frag);

    // (a): parent.children == `[pre_existing, c0, c1]` (append at tail, source order).
    assert_eq!(
        doc.nodes[parent].children,
        vec![pre_existing, c0, c1],
        "attach_child with DocumentFragment must extend parent's children with fragment's children in source order"
    );
}

#[test]
fn attach_child_empties_fragments_children_after_move() {
    // (b): empty the fragment’s children Vec (move semantics, not clone).
    // The old push behavior would retain fragment.children; this test checks the move.
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);

    // Precondition: the fragment itself has two children.
    assert_eq!(doc.nodes[frag].children.len(), 2);

    doc.attach_child(parent, frag);
    assert!(
        doc.nodes[frag].children.is_empty(),
        "fragment's children must be drained after attach_child (move semantics per WHATWG DOM §4.2.3 Mutation algorithms — insert step 4.1)"
    );
    // The fragment itself remains in the arena (kind = DocumentFragment, detached).
    assert!(matches!(doc.nodes[frag].data, NodeData::DocumentFragment));
}

#[test]
fn attach_child_does_not_push_the_fragment_node_itself() {
    // (c): the fragment node itself must never appear in parent.children.
    // WHATWG DOM §4.2.3 Mutation algorithms — insert step 1 defines nodes
    // as fragment.children for a fragment. The fragment itself is excluded from
    // tree insertion (mutation records likewise observe parent → fragment’s
    // children, not parent → fragment).
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);

    doc.attach_child(parent, frag);

    assert!(
        !doc.nodes[parent].children.contains(&frag),
        "fragment node itself must NOT appear in parent.children (spec: fragment is unrendered container)"
    );
}

#[test]
fn attach_child_with_empty_fragment_is_noop_on_parent_children() {
    // (d): attaching an empty fragment does not change the parent’s children.
    // Security perspective (raw-arena-index footgun): with an empty fragment,
    // drain().collect() returns an empty Vec and extend does nothing.
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let existing = doc.append_element(Some(parent), "p", Style::default(), None::<&str>);

    // Empty fragment.
    let empty_frag = doc.nodes.len();
    doc.nodes.push(Node::new_document_fragment());

    doc.attach_child(parent, empty_frag);
    assert_eq!(
        doc.nodes[parent].children,
        vec![existing],
        "empty fragment attach must not change parent.children"
    );
    assert!(!doc.nodes[parent].children.contains(&empty_frag));
}

#[test]
fn attach_child_non_fragment_keeps_existing_push_semantics() {
    // (e): non-fragment children (Element / Text / Comment / PI /
    // Document) retain the old behavior (simple push); the fragment branch
    // must cause no collateral damage.
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    // Element child, detached.
    let elem = doc.append_element(None, "span", Style::default(), None::<&str>);
    doc.attach_child(parent, elem);
    assert_eq!(doc.nodes[parent].children, vec![elem]);
    // Comment child, detached.
    let comment = doc.append_comment(None, "hi");
    doc.attach_child(parent, comment);
    assert_eq!(doc.nodes[parent].children, vec![elem, comment]);
}
