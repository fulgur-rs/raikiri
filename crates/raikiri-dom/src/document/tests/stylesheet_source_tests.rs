use super::*;

fn page() -> (Document, usize, usize, usize) {
    let mut doc = Document::new();
    let head = doc.append_element(Some(doc.root_index()), "head", None::<&str>);
    let style = doc.append_element(Some(head), "style", None::<&str>);
    let text = doc.append_text(style, "p { color: red }");
    doc.mark_in_document_flags();
    (doc, head, style, text)
}

#[test]
fn cssom_source_is_snapshot_state_but_not_dom_clone_state() {
    let (mut doc, _, style, text) = page();
    doc.set_stylesheet_source(style, "p { color: blue }".into())
        .unwrap();
    assert_eq!(
        doc.clone().stylesheet_source(style),
        Some("p { color: blue }")
    );
    let extracted = doc.extract_nodes(&[0, style, text]);
    assert_eq!(extracted.stylesheet_source(1), Some("p { color: blue }"));
    let clone = doc.clone_node(style, true).unwrap();
    assert_eq!(doc.stylesheet_source(clone), None);
    assert_eq!(
        doc.element_text_content(clone).as_deref(),
        Some("p { color: red }")
    );
    assert_eq!(doc.serialize_inner_html(style).unwrap(), "p { color: red }");
    assert!(doc.set_stylesheet_source(text, String::new()).is_err());
    assert!(
        doc.set_stylesheet_source(usize::MAX, String::new())
            .is_err()
    );
}

#[test]
fn cascade_prefers_cssom_source_including_empty_over_dom_text() {
    let (mut doc, _, style, _) = page();
    for source in ["p { color: blue }", ""] {
        doc.set_stylesheet_source(style, source.into()).unwrap();
        let mut actual = Vec::new();
        raikiri_style::walk_style_elements(&doc, |text| actual.push(text.to_owned()));
        assert_eq!(
            actual,
            if source.is_empty() {
                vec![]
            } else {
                vec![source]
            }
        );
    }
}

#[test]
fn native_mutations_reset_only_affected_cssom_sources() {
    let (mut doc, head, style, text) = page();
    let other = doc.append_element(Some(head), "style", None::<&str>);
    doc.set_stylesheet_source(other, "other {}".into()).unwrap();
    type Mutation = Box<dyn Fn(&mut Document)>;
    let actions: Vec<Mutation> = vec![
        Box::new(move |doc| {
            doc.set_character_data(text, "p { color: red }").unwrap();
        }),
        Box::new(move |doc| {
            doc.append_comment(Some(style), "comment");
        }),
        Box::new(move |doc| {
            doc.append_text_coalesced(style, " ");
        }),
        Box::new(move |doc| {
            doc.set_element_text_content(style, "").unwrap();
        }),
        Box::new(move |doc| {
            doc.replace_children_from(style, &Document::new(), 0);
        }),
        Box::new(move |doc| {
            doc.detach_from_parent(head);
        }),
        Box::new(move |doc| {
            doc.reparent_children(style, other);
        }),
        Box::new(move |doc| {
            doc.retain_children(|child| child != style);
        }),
        Box::new(move |doc| {
            doc.set_element_attribute(style, "type", "text/plain")
                .unwrap();
        }),
        Box::new(move |doc| {
            doc.set_element_attributes(style, vec![("type".into(), "text/plain".into())]);
        }),
        Box::new(move |doc| {
            doc.set_element_namespace(style, Some("urn:other".into()));
        }),
    ];
    for action in actions {
        let mut changed = doc.clone();
        changed
            .set_stylesheet_source(style, "p { color: blue }".into())
            .unwrap();
        action(&mut changed);
        assert_eq!(changed.stylesheet_source(style), None);
    }
    doc.set_stylesheet_source(style, "blue {}".into()).unwrap();
    doc.set_element_attribute(style, "class", "unchanged")
        .unwrap();
    doc.set_element_attribute(style, "type", "TEXT/CSS")
        .unwrap();
    doc.set_character_data(text, "p { color: green }").unwrap();
    assert_eq!(doc.stylesheet_source(other), Some("other {}"));
    doc.set_stylesheet_source(style, "blue {}".into()).unwrap();
    doc.set_element_attribute(style, "media", "print").unwrap();
    assert_eq!(doc.stylesheet_source(style), Some("blue {}"));
}

#[test]
fn svg_render_serialization_uses_current_source_with_xml_escaping() {
    let mut doc = Document::new();
    let svg = doc.append_element(Some(0), "svg", None::<&str>);
    doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    let style = doc.append_element(Some(svg), "style", None::<&str>);
    doc.set_element_namespace(style, Some("http://www.w3.org/2000/svg".into()));
    doc.append_text(style, "original {}");
    doc.set_stylesheet_source(style, "text::before { content: '</style>&'; }".into())
        .unwrap();
    let rendered = doc.serialize_svg_subtree(svg).unwrap().unwrap();
    assert!(rendered.contains("&lt;/style&gt;&amp;"));
    assert!(!rendered.contains("original {}"));
    assert!(
        doc.serialize_inner_html(svg)
            .unwrap()
            .contains("original {}")
    );
}

#[test]
fn type_removal_and_insertion_primitives_replace_current_sources() {
    let (mut base, head, style, text) = page();
    base.set_element_attribute(style, "type", "text/plain")
        .unwrap();
    for namespaced in [false, true] {
        let mut doc = base.clone();
        doc.set_stylesheet_source(style, "edited {}".into())
            .unwrap();
        if namespaced {
            doc.remove_element_attribute_ns(style, None, "type");
        } else {
            doc.remove_element_attribute(style, "type").unwrap();
        }
        assert_eq!(doc.stylesheet_source(style), None);
    }
    let mut doc = base;
    let child = doc.append_element(Some(head), "div", None::<&str>);
    doc.set_stylesheet_source(style, "edited {}".into())
        .unwrap();
    doc.insert_text_before_coalesced(style, text, "prefix");
    assert_eq!(doc.stylesheet_source(style), None);
    doc.set_stylesheet_source(style, "edited {}".into())
        .unwrap();
    doc.insert_text_before_coalesced(style, text, "more");
    assert_eq!(doc.stylesheet_source(style), None);
    doc.set_stylesheet_source(style, "edited {}".into())
        .unwrap();
    doc.insert_child_before(style, text, child);
    assert_eq!(doc.stylesheet_source(style), None);
}

#[test]
fn unrelated_leaf_insertions_do_constant_work_with_many_edited_sheets() {
    for count in [64, 256, 1024] {
        let mut doc = Document::new();
        let head = doc.append_element(Some(0), "head", None::<&str>);
        let body = doc.append_element(Some(0), "body", None::<&str>);
        for _ in 0..count {
            let style = doc.append_element(Some(head), "style", None::<&str>);
            doc.set_stylesheet_source(style, "p {}".into()).unwrap();
        }
        super::super::stylesheet_source::take_reset_work();
        for _ in 0..1024 {
            doc.append_text(body, "text");
        }
        assert_eq!(super::super::stylesheet_source::take_reset_work(), 2048);
        assert_eq!(doc.stylesheet_sources.len(), count);
    }
}

#[test]
fn native_text_write_resets_render_source_even_when_text_is_unchanged() {
    let (mut doc, _, style, text) = page();
    doc.set_stylesheet_source(style, "p { color: blue }".into())
        .unwrap();
    doc.set_character_data(text, "p { color: red }").unwrap();
    let mut sources = Vec::new();
    raikiri_style::walk_style_elements(&doc, |source| sources.push(source.to_owned()));
    assert_eq!(sources, ["p { color: red }"]);
}

#[test]
fn cssom_source_keeps_the_generic_walks_descendant_styles_visible() {
    let mut doc = Document::new();
    let outer = doc.append_element(Some(0), "style", None::<&str>);
    let inner = doc.append_element(Some(outer), "style", None::<&str>);
    doc.append_text(inner, "inner {}");
    doc.mark_in_document_flags();
    for source in ["outer {}", ""] {
        doc.set_stylesheet_source(outer, source.into()).unwrap();
        let mut actual = Vec::new();
        raikiri_style::walk_style_elements(&doc, |text| actual.push(text.to_owned()));
        let mut expected = Vec::new();
        if !source.is_empty() {
            expected.push(source);
        }
        expected.push("inner {}");
        assert_eq!(actual, expected);
    }
}
