use super::*;
use crate::node::NodeFlags;
use taffy::TraversePartialTree;

#[test]
fn taffy_child_ids_and_count_filter_out_template_descendants() {
    // Regression pin。taffy layout tree
    // (= web spec flat tree) から template descendants を除外する。
    // template 自身は in_document=true なので body の child 数に含まれる、
    // その内側の <p> は in_document=false なので template の taffy child
    // 数 = 0 になる。
    let mut doc = Document::new();
    let root = doc.root_index();
    let body = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let tmpl = doc.append_element(Some(body), "template", Style::default(), None::<&str>);
    let inner = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
    let _txt = doc.append_text(inner, "hi");

    doc.mark_in_document_flags();

    let body_id = taffy::NodeId::from(body);
    let tmpl_id = taffy::NodeId::from(tmpl);

    // body の直接子は template 1 個 (taffy 経由 count)
    assert_eq!(
        <Document as TraversePartialTree>::child_count(&doc, body_id),
        1,
        "body has template as its one filtered child"
    );
    let body_children: Vec<taffy::NodeId> =
        <Document as TraversePartialTree>::child_ids(&doc, body_id).collect();
    assert_eq!(body_children, vec![tmpl_id]);

    // template の taffy view から見た child_count = 0 (inner <p> は filter される)
    assert_eq!(
        <Document as TraversePartialTree>::child_count(&doc, tmpl_id),
        0,
        "template contents are filtered out of taffy layout tree"
    );
    let tmpl_children: Vec<taffy::NodeId> =
        <Document as TraversePartialTree>::child_ids(&doc, tmpl_id).collect();
    assert!(tmpl_children.is_empty());
}

#[test]
fn synthetic_inline_root_keeps_collapsed_whitespace_child() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
    let whitespace = doc.append_text(parent, "\n  ");
    doc.nodes[parent].style.display = taffy::Display::Flex;
    doc.mark_in_document_flags();

    let parent_id = taffy::NodeId::from(parent);
    assert_eq!(
        <Document as TraversePartialTree>::child_count(&doc, parent_id),
        0,
    );

    doc.nodes[parent].flags.insert(NodeFlags::IS_INLINE_ROOT);
    let children: Vec<taffy::NodeId> =
        <Document as TraversePartialTree>::child_ids(&doc, parent_id).collect();
    assert_eq!(children, vec![taffy::NodeId::from(whitespace)]);
}

#[test]
fn taffy_child_ids_and_count_filter_out_comment_and_pi_variants() {
    // Regression check:
    // Comment / ProcessingInstruction variant を body 直下に attach した後
    // mark_in_document_flags を経由すると、TaffyChildIter は
    // is_in_document filter でこれらを skip する。旧 strip_non_element_stubs
    // が担っていた "layout tree から non-Element node を消す" 機能が、
    // strip 廃止後は「NodeData variant → mark_in_document_flags で
    // IS_IN_DOCUMENT clear → TaffyChildIter が filter」の chain に置き換わって
    // いることを end-to-end で pin。
    //
    // 特に「Comment/PI が layout child count に leak する」
    // failure mode を stress する: body 直下に Comment 3 個 + PI 2 個 + <p>、
    // という mix で、body の taffy child_count == 1 (<p> only) を要求する。
    let mut doc = Document::new();
    let root = doc.root_index();
    let body = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    // Interleaved で attach、ordering に依存しないことを確認。
    let _c0 = doc.append_comment(Some(body), "hello");
    let _pi0 = doc.append_processing_instruction(Some(body), "xml-stylesheet", "href='x'");
    let _c1 = doc.append_comment(Some(body), "middle");
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let _c2 = doc.append_comment(Some(body), "end");
    let _pi1 = doc.append_processing_instruction(Some(body), "xml", "version='1.0'");

    doc.mark_in_document_flags();

    // (a) Comment / PI variant node は IS_IN_DOCUMENT が clear されている。
    for i in 0..doc.node_count() {
        let n = doc.get_node(i).unwrap();
        match n.kind() {
            raikiri_traits::NodeKind::Comment | raikiri_traits::NodeKind::ProcessingInstruction => {
                assert!(
                    !n.is_in_document(),
                    "Comment/PI at arena idx {i} must have IS_IN_DOCUMENT cleared \
                     after mark_in_document_flags"
                );
            }
            _ => {}
        }
    }

    // (b) taffy layout tree から見た body の child は <p> の 1 個のみ。
    let body_taffy = taffy::NodeId::from(body);
    let p_taffy = taffy::NodeId::from(p);
    assert_eq!(
        <Document as TraversePartialTree>::child_count(&doc, body_taffy),
        1,
        "body's taffy child_count must be 1 (only <p>), Comment/PI filtered"
    );
    let kids: Vec<taffy::NodeId> =
        <Document as TraversePartialTree>::child_ids(&doc, body_taffy).collect();
    assert_eq!(kids, vec![p_taffy]);

    // (c) get_child_id も filtered view で consistent (index 0 = <p>)。
    assert_eq!(
        <Document as TraversePartialTree>::get_child_id(&doc, body_taffy, 0),
        p_taffy
    );
}
