use super::*;
use raikiri_style::{build_rule_tree, cascade};
use taffy::Style;

fn contents_chain(body_style: &str) -> (Document, CascadeResult, [usize; 4]) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), Some("display:block"));
    let body = document.append_element(Some(html), "body", Style::default(), Some(body_style));
    let outer = document.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("display:contents"),
    );
    let inner = document.append_element(
        Some(outer),
        "div",
        Style::default(),
        Some("display:contents"),
    );
    let child = document.append_element(Some(inner), "span", Style::default(), None::<&str>);
    document.mark_in_document_flags();
    let cascade = cascade(&document, &build_rule_tree(&document)).unwrap();
    (document, cascade, [html, body, inner, child])
}

#[test]
fn contents_chain_stops_at_a_box_from_the_cascade() {
    let (document, cascade, [_, body, _, child]) = contents_chain("display:block");
    assert_eq!(
        box_parent(&document, &cascade, child, BoxParentScope::Document),
        Some(body)
    );
    assert_eq!(
        box_parent(&document, &cascade, child, BoxParentScope::LayoutRoot),
        Some(body)
    );
}

#[test]
fn only_layout_root_scope_keeps_a_contents_body() {
    let (document, cascade, [html, body, _, child]) = contents_chain("display:contents");
    assert_eq!(
        box_parent(&document, &cascade, child, BoxParentScope::Document),
        Some(html)
    );
    assert_eq!(
        box_parent(&document, &cascade, child, BoxParentScope::LayoutRoot),
        Some(body)
    );
}

#[test]
fn layout_root_scope_stops_at_a_non_element_even_when_its_display_is_contents() {
    let mut document = Document::new();
    let body = document.append_element(Some(0), "body", Style::default(), Some("display:block"));
    let text = document.append_text(body, "boundary");
    // Preserve the IFC boundary even for a manually built non-element parent.
    let child = document.append_element(Some(text), "span", Style::default(), None::<&str>);
    document.mark_in_document_flags();
    let mut cascade = cascade(&document, &build_rule_tree(&document)).unwrap();
    cascade.computed[text].display = DisplayValue::Contents;
    assert_eq!(
        box_parent(&document, &cascade, child, BoxParentScope::LayoutRoot),
        None
    );
    assert_eq!(
        box_parent(&document, &cascade, child, BoxParentScope::Document),
        Some(body)
    );
}

#[test]
fn a_missing_parent_entry_is_a_boundary_for_partial_paint_cascades() {
    let (document, mut cascade, [_, body, inner, child]) = contents_chain("display:block");
    cascade.computed.truncate(body + 1);
    assert_eq!(
        box_parent(&document, &cascade, child, BoxParentScope::Document),
        Some(inner)
    );
}
