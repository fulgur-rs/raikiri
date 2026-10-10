//! `Document::extract_nodes`: copying part of a tree into a new document.

use super::*;
use taffy::Style;

#[test]
fn listed_nodes_are_copied_in_order_with_their_tree_and_data() {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    let before = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    doc.append_text(before, "skipped");
    let kept = doc.append_element(Some(body), "div", Style::default(), None::<&str>);
    let span = doc.append_element(Some(kept), "span", Style::default(), None::<&str>);
    let text = doc.append_text(span, "kept");
    let after = doc.append_element(Some(body), "p", Style::default(), None::<&str>);
    let img = doc.append_element(Some(kept), "img", Style::default(), None::<&str>);
    let url = url::Url::parse("https://example.test/a.png").expect("url");
    doc.resolved_image_urls
        .insert(img, ("a.png".to_owned(), url.clone()));
    doc.resolved_image_urls
        .insert(after, ("b.png".to_owned(), url.clone()));
    let fragment = doc.append_element(None, "template-contents", Style::default(), None::<&str>);
    doc.nodes[kept]
        .data
        .as_element_mut()
        .expect("element")
        .template_contents = Some(fragment);

    let nodes = [0, html, body, kept, span, text, img];
    let mut copy = doc.extract_nodes(&nodes);
    copy.mark_in_document_flags();

    assert_eq!(copy.node_count(), nodes.len());
    assert_eq!(copy.root_index(), 0);
    let children = |id: usize| copy.get_node(id).expect("node").children.clone();
    assert_eq!(children(0), vec![1]);
    assert_eq!(children(1), vec![2]);
    // Only the listed children of <body> remain.
    assert_eq!(children(2), vec![3]);
    assert_eq!(children(3), vec![4, 6]);
    assert_eq!(children(4), vec![5]);
    for (new, &old) in nodes.iter().enumerate().skip(1) {
        let parent = copy.parent_of(new).expect("attached");
        assert_eq!(nodes[parent], doc.parent_of(old).expect("source parent"));
        assert!(copy.get_node(new).expect("node").is_in_document());
    }
    assert!(matches!(
        &copy.get_node(5).expect("text").data,
        NodeData::Text(data) if data.text_content.as_str() == "kept"
    ));
    // Per-node side data follows the node; data of nodes left out does not.
    assert_eq!(
        copy.resolved_image_urls.keys().copied().collect::<Vec<_>>(),
        vec![6]
    );
    assert_eq!(copy.resolved_image_urls[&6].1, url);
    // `<template>` contents live outside the tree and are not copied.
    assert_eq!(
        copy.get_node(3)
            .and_then(|node| match &node.data {
                NodeData::Element(element) => Some(element.template_contents),
                _ => None,
            })
            .expect("element"),
        None
    );
    assert!(copy.layout_dirty);
}

#[test]
fn a_node_listed_without_its_parent_is_detached() {
    let mut doc = Document::new();
    let outer = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let inner = doc.append_element(Some(outer), "span", Style::default(), None::<&str>);
    let mut copy = doc.extract_nodes(&[0, inner]);
    copy.mark_in_document_flags();
    assert_eq!(copy.parent_of(1), None);
    assert!(copy.get_node(0).expect("root").children.is_empty());
    assert!(!copy.get_node(1).expect("node").is_in_document());
}
