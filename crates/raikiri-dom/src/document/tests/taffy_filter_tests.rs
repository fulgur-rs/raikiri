use super::*;
use crate::node::NodeFlags;
use taffy::TraversePartialTree;

#[test]
fn taffy_child_ids_and_count_filter_out_template_descendants() {
    // Regression check: exclude template descendants from the taffy layout tree
    // (= the web-spec flat tree).
    // The template itself has in_document=true and counts as a body child,
    // but its inner <p> has in_document=false, so the template has zero
    // taffy children.
    let mut doc = Document::new();
    let root = doc.root_index();
    let body = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let tmpl = doc.append_element(Some(body), "template", Style::default(), None::<&str>);
    let inner = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
    let _txt = doc.append_text(inner, "hi");

    doc.mark_in_document_flags();

    let body_id = taffy::NodeId::from(body);
    let tmpl_id = taffy::NodeId::from(tmpl);

    // The body directly contains one template child (taffy count).
    assert_eq!(
        <Document as TraversePartialTree>::child_count(&doc, body_id),
        1,
        "body has template as its one filtered child"
    );
    let body_children: Vec<taffy::NodeId> =
        <Document as TraversePartialTree>::child_ids(&doc, body_id).collect();
    assert_eq!(body_children, vec![tmpl_id]);

    // The template has taffy child_count = 0 (its inner <p> is filtered).
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
    // After attaching Comment / ProcessingInstruction variants directly to the body
    // and calling mark_in_document_flags, TaffyChildIter skips these nodes
    // through the is_in_document filter. The former strip_non_element_stubs
    // removed non-Element nodes from the layout tree; after removing that strip,
    // the chain is “NodeData variant → mark_in_document_flags clears
    // IS_IN_DOCUMENT → TaffyChildIter filters.” This test pins that chain
    // end to end.
    //
    // In particular, stress the failure mode where Comments / PIs leak into
    // the layout child count: mix three Comments, two PIs, and one <p> directly
    // under body and require body taffy child_count == 1 (only <p>).
    let mut doc = Document::new();
    let root = doc.root_index();
    let body = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    // Attach in interleaved order; verify order independence.
    let _c0 = doc.append_comment(Some(body), "hello");
    let _pi0 = doc.append_processing_instruction(Some(body), "xml-stylesheet", "href='x'");
    let _c1 = doc.append_comment(Some(body), "middle");
    let p = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let _c2 = doc.append_comment(Some(body), "end");
    let _pi1 = doc.append_processing_instruction(Some(body), "xml", "version='1.0'");

    doc.mark_in_document_flags();

    // (a) Clear IS_IN_DOCUMENT on Comment / PI variant nodes.
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

    // (b) The body has only one child (<p>) in the taffy layout tree.
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

    // (c) get_child_id also agrees with the filtered view (index 0 = <p>).
    assert_eq!(
        <Document as TraversePartialTree>::get_child_id(&doc, body_taffy, 0),
        p_taffy
    );
}
