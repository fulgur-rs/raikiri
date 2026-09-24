//! attach_child が
//! `NodeData::DocumentFragment` を child に受け取った時、WHATWG DOM §4.2.3
//! Mutation algorithms — insert algorithm steps 1 + 4.1 + 7.2 と一致する
//! fragment-aware semantics で動作する契約を check (append が positional
//! splice の step 7.3 ではなく 7.2 に対応する導出は `Document::attach_child`
//! の doc comment 参照)。
//!
//! Spec ref:
//! - <https://dom.spec.whatwg.org/#concept-node-pre-insert>
//! - <https://dom.spec.whatwg.org/#concept-node-insert>
//!
//! 契約 (test 5 分割):
//! (a) parent.children が fragment の children で source order に extend される
//! (b) fragment の children Vec が empty 化される (move、not clone)
//! (c) fragment node 自身は parent.children に含まれない
//! (d) empty fragment attach は parent.children を変えない (edge)
//! (e) fragment 以外の child は旧 push 挙動を維持する (regression check)
use super::*;
use crate::node::NodeData;

fn make_fragment_with_two_children(doc: &mut Document) -> (usize, usize, usize) {
    let frag = doc.nodes.len();
    doc.nodes.push(Node::new_document_fragment());
    // fragment の children は detached の Element 2 個。
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
    // parent には先に既存 child を 1 個入れておく。
    let pre_existing = doc.append_element(Some(parent), "pre", Style::default(), None::<&str>);

    let (frag, c0, c1) = make_fragment_with_two_children(&mut doc);
    doc.attach_child(parent, frag);

    // (a): parent.children == `[pre_existing, c0, c1]` (append at tail, source order)。
    assert_eq!(
        doc.nodes[parent].children,
        vec![pre_existing, c0, c1],
        "attach_child with DocumentFragment must extend parent's children with fragment's children in source order"
    );
}

#[test]
fn attach_child_empties_fragments_children_after_move() {
    // (b): fragment の children Vec は空になる (move semantics、clone ではない)。
    // 旧 push 挙動なら fragment.children は保たれるので、この test が move を check する。
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);

    // 事前確認: fragment 自身は 2 個の children を持つ。
    assert_eq!(doc.nodes[frag].children.len(), 2);

    doc.attach_child(parent, frag);
    assert!(
        doc.nodes[frag].children.is_empty(),
        "fragment's children must be drained after attach_child (move semantics per WHATWG DOM §4.2.3 Mutation algorithms — insert step 4.1)"
    );
    // fragment 自身は arena には残る (kind = DocumentFragment、detached)。
    assert!(matches!(doc.nodes[frag].data, NodeData::DocumentFragment));
}

#[test]
fn attach_child_does_not_push_the_fragment_node_itself() {
    // (c): fragment node 自身は parent.children に絶対に含まれない。
    // WHATWG DOM §4.2.3 Mutation algorithms — insert step 1 は fragment の
    // 場合 nodes = fragment.children と定義し、fragment 自身は tree
    // insertion 対象外となる (mutation record 上も
    // parent → fragment ではなく parent → fragment's children で観測される)。
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
    // (d): empty fragment attach は parent の children を変えない。
    // security lens (raw-arena-index footgun): empty fragment で
    // drain().collect() が空 Vec を返し extend が何もしないことを pin。
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let existing = doc.append_element(Some(parent), "p", Style::default(), None::<&str>);

    // Empty fragment。
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
    // (e): fragment 以外 (Element / Text / Comment / PI /
    // Document) は旧 挙動 (単純 push) 継続、fragment 分岐が collateral damage
    // を出さないことを pin。
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    // Element child, detached。
    let elem = doc.append_element(None, "span", Style::default(), None::<&str>);
    doc.attach_child(parent, elem);
    assert_eq!(doc.nodes[parent].children, vec![elem]);
    // Comment child, detached。
    let comment = doc.append_comment(None, "hi");
    doc.attach_child(parent, comment);
    assert_eq!(doc.nodes[parent].children, vec![elem, comment]);
}
