use super::*;
use crate::node::NodeFlags;
use taffy::Style;

#[test]
fn replace_children_from_deep_copies_nodes_in_order_and_detaches_old_subtree() {
    let mut target = Document::new();
    let target_parent = target.append_element(Some(0), "main", Style::default(), None::<&str>);
    let old = target.append_element(Some(target_parent), "old", Style::default(), None::<&str>);
    let old_text = target.append_text(old, "kept in arena");

    let mut source = Document::new();
    let source_parent = source.append_element(Some(0), "source", Style::default(), None::<&str>);
    let source_comment = source.append_comment(Some(source_parent), "leading comment");
    let source_instruction =
        source.append_processing_instruction(Some(source_parent), "work", "ready");
    let section = source.append_element(
        Some(source_parent),
        "section",
        Style::default(),
        Some("color: red"),
    );
    source.set_element_namespace(section, Some(SmolStr::new("urn:example")));
    source.set_element_attributes(
        section,
        vec![
            (SmolStr::new("id"), SmolStr::new("copied")),
            (SmolStr::new("data-key"), SmolStr::new("value")),
        ],
    );
    let text = source.append_text(section, "first");
    let comment = source.append_comment(Some(section), "middle comment");
    let span = source.append_element(Some(section), "span", Style::default(), None::<&str>);
    source.set_element_attributes(span, vec![(SmolStr::new("title"), SmolStr::new("tip"))]);
    source
        .set_element_attribute(span, "style", "display: block")
        .unwrap();
    let trailing_text = source.append_text(section, "last");

    // Reset the target flags to prove this operation routes mutations
    // through the normal invalidation paths.
    target.layout_dirty = false;
    target.flags_dirty = false;
    target.replace_children_from(target_parent, &source, source_parent);

    assert!(target.layout_dirty);
    assert!(target.flags_dirty);
    assert_eq!(target.nodes[target_parent].children.len(), 3);
    let copied_comment = target.nodes[target_parent].children[0];
    let copied_instruction = target.nodes[target_parent].children[1];
    let copied_section = target.nodes[target_parent].children[2];
    assert_ne!(copied_comment, source_comment);
    assert_ne!(copied_instruction, source_instruction);
    assert_ne!(copied_section, section);
    assert!(matches!(
        target.nodes[copied_comment].data,
        NodeData::Comment(_)
    ));
    assert!(matches!(
        &target.nodes[copied_instruction].data,
        NodeData::ProcessingInstruction { target, data }
            if target == "work" && data == "ready"
    ));
    assert_eq!(
        target.parent_of(old),
        None,
        "replaced nodes stay allocated but detached" // cov:ignore: assert_eq! formats this diagnostic only on failure.
    );
    assert_eq!(
        target.nodes[old].children,
        vec![old_text],
        "the old subtree remains allocated" // cov:ignore: assert_eq! formats this diagnostic only on failure.
    );
    assert_eq!(target.parent_of(old_text), Some(old));

    let NodeData::Element(copied_element) = &target.nodes[copied_section].data else {
        panic!("expected copied section element"); // cov:ignore: the source fixture constructs this as an Element.
    };
    assert_eq!(copied_element.tag_name.as_str(), "section");
    assert_eq!(copied_element.namespace.as_deref(), Some("urn:example"));
    assert_eq!(copied_element.inline_style.as_deref(), Some("color: red"));
    assert_eq!(
        copied_element
            .attributes
            .iter()
            .map(|attr| (attr.local.as_str(), attr.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("id", "copied"), ("data-key", "value")]
    );

    let copied_children = target.nodes[copied_section].children.clone();
    assert_eq!(copied_children.len(), 4);
    let copied_text = copied_children[0];
    let copied_comment = copied_children[1];
    let copied_span = copied_children[2];
    let copied_trailing_text = copied_children[3];
    assert_ne!(copied_text, text);
    assert_ne!(copied_comment, comment);
    assert_ne!(copied_trailing_text, trailing_text);
    assert!(
        matches!(&target.nodes[copied_text].data, NodeData::Text(t) if t.text_content == "first")
    );
    assert!(
        matches!(&target.nodes[copied_comment].data, NodeData::Comment(t) if t == "middle comment")
    );
    assert!(
        matches!(&target.nodes[copied_trailing_text].data, NodeData::Text(t) if t.text_content == "last")
    );
    let NodeData::Element(copied_span_data) = &target.nodes[copied_span].data else {
        panic!("expected copied span element"); // cov:ignore: the source fixture constructs this as an Element.
    };
    assert_eq!(
        copied_span_data.inline_style.as_deref(),
        Some("display: block")
    );
    assert_eq!(copied_span_data.attributes.len(), 1);
    assert_eq!(copied_span_data.attributes[0].local.as_str(), "title");
    assert_eq!(copied_span_data.attributes[0].value.as_str(), "tip");

    target.mark_in_document_flags();
    assert!(!target.nodes[old].flags.contains(NodeFlags::IS_IN_DOCUMENT));
    assert!(
        !target.nodes[old_text]
            .flags
            .contains(NodeFlags::IS_IN_DOCUMENT)
    );
    assert!(
        target.nodes[copied_section]
            .flags
            .contains(NodeFlags::IS_IN_DOCUMENT)
    );
    assert!(
        !target.nodes[copied_comment]
            .flags
            .contains(NodeFlags::IS_IN_DOCUMENT)
    );
    // Copying does not mutate or consume source children.
    assert_eq!(
        source.nodes[source_parent].children,
        vec![source_comment, source_instruction, section]
    );
}

#[test]
fn replace_children_from_splices_document_fragment_children() {
    let mut source = Document::new();
    let source_parent = source.append_element(Some(0), "section", Style::default(), None::<&str>);
    let fragment = source.nodes.len();
    source.nodes.push(Node::new_document_fragment());
    source.nodes[source_parent].children.push(fragment);
    source.append_text(fragment, "fragment text");

    let mut target = Document::new();
    let target_parent = target.append_element(Some(0), "main", Style::default(), None::<&str>);
    target.replace_children_from(target_parent, &source, source_parent);
    assert_eq!(
        target.serialize_inner_html(target_parent).unwrap(),
        "fragment text"
    );
}

#[test]
#[should_panic(expected = "replace_children_from cannot copy a Document node as a child")]
fn replace_children_from_rejects_document_nodes_in_child_lists() {
    let mut source = Document::new();
    let source_parent = source.append_element(Some(0), "section", Style::default(), None::<&str>);
    let source_root = source.root_index();
    source.nodes[source_parent].children.push(source_root);

    let mut target = Document::new();
    let target_parent = target.append_element(Some(0), "main", Style::default(), None::<&str>);
    target.replace_children_from(target_parent, &source, source_parent);
}
