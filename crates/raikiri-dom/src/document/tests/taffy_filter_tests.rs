use super::*;
use crate::node::NodeFlags;
use raikiri_style::property::DisplayValue;
use taffy::TraversePartialTree;

#[test]
fn taffy_children_exclude_non_rendered_html_elements() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let html = doc.append_element(Some(root), "html", Style::default(), None::<&str>);
    let head = doc.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = doc.append_element(Some(head), "style", Style::default(), None::<&str>);
    doc.append_text(style, "body { margin: 0 }");
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let content = doc.append_element(Some(body), "div", Style::default(), None::<&str>);

    doc.mark_in_document_flags();

    let html_id = taffy::NodeId::from(html);
    assert_eq!(
        <Document as TraversePartialTree>::child_count(&doc, html_id),
        1,
        "html layout must exclude head and retain body"
    );
    let children: Vec<taffy::NodeId> =
        <Document as TraversePartialTree>::child_ids(&doc, html_id).collect();
    assert_eq!(children, vec![taffy::NodeId::from(body)]);

    let body_children: Vec<taffy::NodeId> =
        <Document as TraversePartialTree>::child_ids(&doc, taffy::NodeId::from(body)).collect();
    assert_eq!(body_children, vec![taffy::NodeId::from(content)]);
}

#[test]
fn taffy_child_ids_preserve_inline_whitespace_and_match_count_and_index() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let parent = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
    let _leading = doc.append_text(parent, "\n  ");
    let left_text = doc.append_text(parent, "left");
    let text_separator = doc.append_text(parent, " ");
    let left = doc.append_element(Some(parent), "span", Style::default(), None::<&str>);
    let separator = doc.append_text(parent, " ");
    let right = doc.append_element(Some(parent), "span", Style::default(), None::<&str>);
    let trailing_separator = doc.append_text(parent, " ");
    let right_text = doc.append_text(parent, "right");
    let _trailing = doc.append_text(parent, "\n  ");
    doc.nodes[parent].style.display = taffy::Display::Block;
    doc.nodes[left].display = DisplayValue::Inline;
    doc.nodes[right].display = DisplayValue::Inline;

    doc.mark_in_document_flags();

    let parent_id = taffy::NodeId::from(parent);
    let expected = vec![
        taffy::NodeId::from(left_text),
        taffy::NodeId::from(text_separator),
        taffy::NodeId::from(left),
        taffy::NodeId::from(separator),
        taffy::NodeId::from(right),
        taffy::NodeId::from(trailing_separator),
        taffy::NodeId::from(right_text),
    ];
    assert_eq!(
        <Document as TraversePartialTree>::child_count(&doc, parent_id),
        expected.len(),
    );
    let actual: Vec<taffy::NodeId> =
        <Document as TraversePartialTree>::child_ids(&doc, parent_id).collect();
    assert_eq!(actual, expected);
    for (index, child) in expected.into_iter().enumerate() {
        assert_eq!(
            <Document as TraversePartialTree>::get_child_id(&doc, parent_id, index),
            child,
        );
    }
}

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
    let body = doc.append_element(Some(root), "body", Style::default(), None::<&str>);
    let tmpl = doc.append_element(Some(body), "template", Style::default(), None::<&str>);
    let inner = doc.append_element(Some(tmpl), "p", Style::default(), None::<&str>);
    let _txt = doc.append_text(inner, "hi");
    let frag = doc.allocate_template_fragment_root(tmpl);
    let frag_span = doc.append_element(Some(frag), "span", Style::default(), None::<&str>);

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

    // The template's real <p> child is part of the flat tree.
    assert_eq!(
        <Document as TraversePartialTree>::child_count(&doc, tmpl_id),
        1,
        "template real children stay in the taffy layout tree"
    );
    let tmpl_children: Vec<taffy::NodeId> =
        <Document as TraversePartialTree>::child_ids(&doc, tmpl_id).collect();
    assert_eq!(tmpl_children, vec![taffy::NodeId::from(inner)]);

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
