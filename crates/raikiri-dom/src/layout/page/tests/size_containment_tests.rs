use super::*;

fn document_with(css: &str) -> (Document, CascadeResult, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let node = doc.append_element(Some(body), "div", Some(css));
    let child = doc.append_element(Some(node), "div", Some("display:block;page:inner"));
    doc.append_text(child, "content");
    doc.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).expect("cascade");
    (doc, cascade, node)
}

#[test]
fn active_full_size_containment_agrees_before_and_after_layout_metadata_exists() {
    for (css, expected) in [
        ("display:block;contain:size", true),
        ("display:block;contain:strict", true),
        ("display:block;contain:content", false),
        ("display:block;contain:layout", false),
        ("display:block;contain:inline-size", false),
        (
            "display:block;writing-mode:vertical-rl;contain:inline-size",
            false,
        ),
        ("display:contents;contain:size", false),
        ("display:none;contain:size", false),
        ("display:inline;contain:size", false),
        ("display:inline-block;contain:size", true),
        ("display:table;contain:size", false),
        ("display:table-cell;contain:size", false),
    ] {
        let (mut doc, cascade, node) = document_with(css);
        let before = size_contained_for_pagination(BoxTree::for_cascade(&doc, &cascade), node);
        crate::layout::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
        let after = size_contained_for_pagination(BoxTree::for_layout(&doc), node);
        assert_eq!((before, after), (expected, expected), "{css}");
    }
}

#[test]
fn a_monolithic_contained_box_does_not_propagate_its_descendants_page_name() {
    for (css, expected) in [
        ("display:block;contain:size;height:50px", None),
        (
            "display:block;contain:inline-size;height:50px",
            Some("inner"),
        ),
    ] {
        let (mut doc, cascade, _) = document_with(css);
        let before = first_page_name(&doc, &cascade);
        crate::layout::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
        let after = first_page_name(&doc, &cascade);
        assert_eq!(before.as_deref(), expected, "before: {css}");
        assert_eq!(after.as_deref(), expected, "after: {css}");
    }
}

fn inline_item(parent_style: &str) -> (Document, CascadeResult, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let parent = doc.append_element(Some(body), "div", Some(parent_style));
    let item = doc.append_element(Some(parent), "span", Some("contain:size;height:50px"));
    let child = doc.append_element(Some(item), "span", Some("display:block;page:inner"));
    doc.append_text(child, "content");
    doc.mark_in_document_flags();
    let cascade =
        raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).expect("cascade");
    (doc, cascade, parent, item)
}

#[test]
fn containment_of_a_blockified_inline_item_uses_the_initial_cascade_parent() {
    for display in ["flex", "grid"] {
        let (mut doc, cascade, _, item) = inline_item(&format!("display:{display}"));
        let before = size_contained_for_pagination(BoxTree::for_cascade(&doc, &cascade), item);
        crate::layout::apply_computed_to_style(&mut doc, &cascade).expect("bridge");
        let after = size_contained_for_pagination(BoxTree::for_layout(&doc), item);
        assert_eq!((before, after), (true, true), "{display}");
    }
}

#[test]
fn a_selected_cascade_does_not_inherit_retained_parent_blockification() {
    for (old, selected, expected) in [
        ("flex", "block", false),
        ("block", "grid", true),
        ("grid", "contents", false),
    ] {
        let (mut doc, original, parent, item) = inline_item(&format!("display:{old}"));
        crate::layout::apply_computed_to_style(&mut doc, &original).expect("original bridge");
        doc.set_element_inline_style(parent, Some(format!("display:{selected}").into()));
        let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc))
            .expect("selected cascade");
        let before = size_contained_for_pagination(BoxTree::for_cascade(&doc, &cascade), item);
        crate::layout::apply_computed_to_style(&mut doc, &cascade).expect("selected bridge");
        let after = size_contained_for_pagination(BoxTree::for_layout(&doc), item);
        assert_eq!((before, after), (expected, expected), "{old} to {selected}");
    }
}
