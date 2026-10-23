use super::*;
use crate::layout::context::LayoutSession;
use taffy::NodeId as TaffyNodeId; // taffy-boundary-lint:ignore: the public Document tree traits take Taffy's id
use taffy::TraversePartialTree;

#[test]
fn taffy_child_ids_keep_template_real_children_but_not_contents_fragment() {
    // Regression check: the taffy layout tree (= the web-spec flat tree)
    // contains a `<template>` element's ordinary light-DOM children but not
    // its detached contents fragment.
    // The template itself has in_document=true and counts as a body child;
    // its real inner <p> is likewise in-document, so the template has one
    // taffy child. The contents fragment is unreachable from the Document
    // root, so its <span> never appears in any taffy child list.
    let mut doc = Document::new();
    let root = doc.root_index();
    let body = doc.append_element(Some(root), "body", None::<&str>);
    let tmpl = doc.append_element(Some(body), "template", None::<&str>);
    let inner = doc.append_element(Some(tmpl), "p", None::<&str>);
    let _txt = doc.append_text(inner, "hi");
    let frag = doc.allocate_template_fragment_root(tmpl);
    let frag_span = doc.append_element(Some(frag), "span", None::<&str>);

    doc.mark_in_document_flags();

    let body_id = TaffyNodeId::from(body);
    let tmpl_id = TaffyNodeId::from(tmpl);
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let session = LayoutSession::bridged(&mut doc, &session_cascade);

    // The body directly contains one template child (taffy count).
    assert_eq!(
        <LayoutSession<'_> as TraversePartialTree>::child_count(&session, body_id),
        1,
        "body has template as its one filtered child"
    );
    let body_children: Vec<TaffyNodeId> =
        <LayoutSession<'_> as TraversePartialTree>::child_ids(&session, body_id).collect();
    assert_eq!(body_children, vec![tmpl_id]);

    // The template's real <p> child is part of the flat tree.
    assert_eq!(
        <LayoutSession<'_> as TraversePartialTree>::child_count(&session, tmpl_id),
        1,
        "template real children stay in the taffy layout tree"
    );
    let tmpl_children: Vec<TaffyNodeId> =
        <LayoutSession<'_> as TraversePartialTree>::child_ids(&session, tmpl_id).collect();
    assert_eq!(tmpl_children, vec![TaffyNodeId::from(inner)]);
    drop(session);

    // The contents fragment subtree appears in no reachable child list.
    assert!(
        !doc.get_node(frag).unwrap().is_in_document(),
        "contents fragment root stays out-of-document"
    );
    assert!(
        !doc.get_node(frag_span).unwrap().is_in_document(),
        "contents fragment child stays out-of-document"
    );
}

#[test]
fn a_flex_container_drops_a_white_space_only_child() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "div", None::<&str>);
    doc.append_text(parent, "\n  ");
    doc.set_display(parent, raikiri_style::property::DisplayValue::Flex);
    doc.mark_in_document_flags();

    let parent_id = TaffyNodeId::from(parent);
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let session = LayoutSession::bridged(&mut doc, &session_cascade);
    assert_eq!(
        <LayoutSession<'_> as TraversePartialTree>::child_count(&session, parent_id),
        0,
    );
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
    let body = doc.append_element(Some(root), "body", None::<&str>);
    // Attach in interleaved order; verify order independence.
    let _c0 = doc.append_comment(Some(body), "hello");
    let _pi0 = doc.append_processing_instruction(Some(body), "xml-stylesheet", "href='x'");
    let _c1 = doc.append_comment(Some(body), "middle");
    let p = doc.append_element(Some(body), "p", None::<&str>);
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
    let body_taffy = TaffyNodeId::from(body);
    let p_taffy = TaffyNodeId::from(p);
    let session_cascade = crate::layout::test_support::cascaded(&doc);
    let session = LayoutSession::bridged(&mut doc, &session_cascade);
    assert_eq!(
        <LayoutSession<'_> as TraversePartialTree>::child_count(&session, body_taffy),
        1,
        "body's taffy child_count must be 1 (only <p>), Comment/PI filtered"
    );
    let kids: Vec<TaffyNodeId> =
        <LayoutSession<'_> as TraversePartialTree>::child_ids(&session, body_taffy).collect();
    assert_eq!(kids, vec![p_taffy]);

    // (c) get_child_id also agrees with the filtered view (index 0 = <p>).
    assert_eq!(
        <LayoutSession<'_> as TraversePartialTree>::get_child_id(&session, body_taffy, 0),
        p_taffy
    );
}

#[test]
fn document_child_indexing_matches_filtered_flat_tree_order() {
    let mut doc = Document::new();
    let parent = doc.append_element(Some(doc.root_index()), "div", None::<&str>);
    let first = doc.append_element(Some(parent), "p", None::<&str>);
    doc.append_text(parent, " \n");
    doc.append_comment(Some(parent), "ignored");
    doc.append_processing_instruction(Some(parent), "xml-stylesheet", "href='x'");
    let contents = doc.append_element(Some(parent), "span", None::<&str>);
    doc.set_display(contents, raikiri_style::property::DisplayValue::Contents);
    let nested_first = doc.append_element(Some(contents), "div", None::<&str>);
    let nested_second = doc.append_element(Some(contents), "div", None::<&str>);
    let last = doc.append_element(Some(parent), "section", None::<&str>);
    doc.mark_in_document_flags();

    let parent_id = TaffyNodeId::from(parent);
    let expected = [first, nested_first, nested_second, last].map(TaffyNodeId::from);
    let children: Vec<_> = <Document as TraversePartialTree>::child_ids(&doc, parent_id).collect();

    assert_eq!(children, expected);
    assert_eq!(
        <Document as TraversePartialTree>::child_count(&doc, parent_id),
        expected.len()
    );
    for (index, child) in expected.into_iter().enumerate() {
        assert_eq!(
            <Document as TraversePartialTree>::get_child_id(&doc, parent_id, index),
            child,
            "Document child index {index}"
        );
    }
}
