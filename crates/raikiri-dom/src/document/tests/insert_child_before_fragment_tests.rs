//! `insert_child_before` が
//! [`NodeData::DocumentFragment`] を child に受け取った時、fragment の
//! children を parent.children の `before` position から source order で
//! splice する fragment-aware semantics (WHATWG DOM §4.2.3 Mutation
//! algorithms — insert algorithm steps 1 + 4.1 + 7.3) を pin。
//! attach_child (tail append) の positional 対応で、これまで latent
//! だった asymmetry を解消する。
//!
//! Spec ref:
//! - <https://dom.spec.whatwg.org/#concept-node-pre-insert>
//! - <https://dom.spec.whatwg.org/#concept-node-insert>
//!
//! 契約 (test 5 分割 = `attach_child_fragment_tests` の mirror):
//! (a) parent.children が fragment の children で `before` position から
//!     source order で splice される
//! (b) fragment の children Vec が empty 化される (move、not clone)
//! (c) fragment node 自身は parent.children に含まれない
//! (d) empty fragment splice は parent.children を変えない (edge)
//! (e) fragment 以外の child は旧 insert 挙動を維持する (regression check)
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
fn insert_child_before_splices_fragment_children_at_position() {
    // (a): parent.children の `before` position に fragment の children が
    // source order で挿入される。tail append の attach_child と違い、
    // positional な splice を check する (`before` の直前に fragment children
    // 全部、その後 `before` 自体、以降既存 sibling が続く)。
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    // parent には既に 2 個 sibling がある: `[first, last]`。
    let first = doc.append_element(Some(parent), "h1", Style::default(), None::<&str>);
    let last = doc.append_element(Some(parent), "footer", Style::default(), None::<&str>);

    let (frag, c0, c1) = make_fragment_with_two_children(&mut doc);
    doc.insert_child_before(parent, last, frag);

    // fragment children c0, c1 が last の前 (= first と last の間) に挿入される。
    assert_eq!(
        doc.nodes[parent].children,
        vec![first, c0, c1, last],
        "insert_child_before with DocumentFragment must splice fragment's children at the `before` position in source order"
    );
}

#[test]
fn insert_child_before_empties_fragments_children_after_move() {
    // (b): fragment の children Vec は空になる (move semantics、clone ではない)。
    // 旧挙動 (fragment 自身を単純 insert) なら fragment.children は保たれる
    // ので、この test が drain の move semantics を check する。
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let sibling = doc.append_element(Some(parent), "p", Style::default(), None::<&str>);

    let (frag, _c0, _c1) = make_fragment_with_two_children(&mut doc);
    // 事前確認: fragment 自身は 2 個の children を持つ。
    assert_eq!(doc.nodes[frag].children.len(), 2);

    doc.insert_child_before(parent, sibling, frag);
    assert!(
        doc.nodes[frag].children.is_empty(),
        "fragment's children must be drained after insert_child_before (move semantics per WHATWG DOM §4.2.3 Mutation algorithms — insert step 4.1)"
    );
    // fragment 自身は arena には残る (kind = DocumentFragment、detached)。
    assert!(matches!(doc.nodes[frag].data, NodeData::DocumentFragment));
}

#[test]
fn insert_child_before_does_not_insert_the_fragment_node_itself() {
    // (c): fragment node 自身は parent.children に絶対に含まれない。
    // WHATWG DOM §4.2.3 Mutation algorithms — insert step 1 は fragment の場合
    // nodes = fragment.children と定義し、fragment 自身は tree insertion 対象外
    // となる (mutation record 上も parent → fragment ではなく
    // parent → fragment's children で観測される)。
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
    // (d): empty fragment splice は parent の children を変えない。
    // splice(pos..pos, empty_vec) が何もしないことを check (attach_child edge
    // check の positional 対応)。
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let a = doc.append_element(Some(parent), "a", Style::default(), None::<&str>);
    let b = doc.append_element(Some(parent), "b", Style::default(), None::<&str>);

    // Empty fragment。
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
    // (e) (acceptance 4): fragment 以外 (Element / Text / Comment
    // / PI / Document) は旧挙動 (単純 insert at `before` position) 継続、
    // fragment 分岐が collateral damage を出さないことを pin。
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let last = doc.append_element(Some(parent), "z", Style::default(), None::<&str>);
    // Detached Element を before=last で insert (foster parenting の primitive)。
    let elem = doc.append_element(None, "m", Style::default(), None::<&str>);
    doc.insert_child_before(parent, last, elem);
    assert_eq!(doc.nodes[parent].children, vec![elem, last]);
}
