use super::*;

fn id(index: usize) -> StyleNodeId {
    StyleNodeId::new(index as u64)
}

#[test]
fn accessors_reflect_the_arena() {
    let mut doc = GenDoc::new(StyleQuirksMode::Quirks);
    let mut div = GenNode::element("div");
    div.attrs.push(("id".into(), "a".into()));
    div.attrs.push(("id".into(), "shadowed".into()));
    div.style = Some("color: red".into());
    div.animation = Some("opacity: 0".into());
    let div = doc.append(0, div);
    let text = doc.append(div, GenNode::text("t"));
    let comment = doc.append(div, GenNode::comment("c"));

    assert_eq!(doc.node_count(), 4);
    assert_eq!(doc.quirks_mode(), StyleQuirksMode::Quirks);
    assert_eq!(doc.root_id(), id(0));
    assert_eq!(doc.parent_id(id(text)), Some(id(div)));
    assert_eq!(doc.parent_id(doc.root_id()), None);
    assert_eq!(doc.parent_id(id(99)), None);
    assert!(doc.node(id(99)).is_none());
    assert_eq!(doc.child_ids(id(99)).count(), 0);
    assert_eq!(
        doc.child_ids(id(div)).collect::<Vec<_>>(),
        vec![id(text), id(comment)]
    );

    let div_node = doc.node(id(div)).expect("div");
    assert_eq!(div_node.kind(), StyleNodeKind::Element);
    assert_eq!(div_node.text_content(), None);
    assert!(div_node.is_in_document());
    let element = div_node.as_element().expect("element");
    assert_eq!(element.tag_name(), "div");
    assert_eq!(element.attr("id"), Some("a"), "the first attribute wins");
    assert_eq!(element.id(), Some("a"));
    assert_eq!(element.attr("style"), Some("color: red"));
    assert_eq!(element.attr("missing"), None);
    assert_eq!(element.inline_style_source(), Some("color: red"));
    assert_eq!(element.animation_style_source(), Some("opacity: 0"));
    assert_eq!(element.namespace_uri(), None);

    let text_node = doc.node(id(text)).expect("text");
    assert_eq!(text_node.kind(), StyleNodeKind::Text);
    assert_eq!(text_node.text_content(), Some("t"));
    assert!(text_node.as_element().is_none());
    assert_eq!(
        doc.node(id(comment)).expect("comment").kind(),
        StyleNodeKind::Comment
    );
}

#[test]
fn detached_parents_detach_their_children() {
    let mut doc = GenDoc::new(StyleQuirksMode::NoQuirks);
    let mut svg = GenNode::element("svg");
    svg.namespace = Some(SVG_NAMESPACE);
    svg.in_document = false;
    let svg = doc.append(0, svg);
    let g = doc.append(svg, GenNode::element("g"));
    let svg_node = doc.node(id(svg)).expect("svg");
    assert!(!svg_node.is_in_document());
    assert_eq!(
        svg_node.as_element().expect("element").namespace_uri(),
        Some(SVG_NAMESPACE)
    );
    assert!(!doc.node(id(g)).expect("g").is_in_document());
}
