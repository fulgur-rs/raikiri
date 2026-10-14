// Regression check: every body lookup (layout, paint and the layout depth
// check) must not select a <body> that lives inside an inert subtree
// (<template>...<body>ghost</body>...</template>).
use super::*;
use crate::layout::find_body as layout_find_body;
use crate::layout::{MAX_LAYOUT_DEPTH, validate_layout_depth};
use crate::paint_rules::find_paint_root;
use raikiri_traits::LayoutError;

/// A document whose `<template>` contents hold a ghost body before the real
/// one, returning `(document, ghost body, real body)`.
fn document_with_template_ghost_body() -> (Document, usize, usize) {
    let mut doc = Document::new();
    let root = doc.root_index();
    let html = doc.append_element(Some(root), "html", Style::default(), None::<&str>);
    // Ghost body in the template's detached contents fragment (should NOT be
    // picked). The fragment is unreachable from the Document root, matching
    // the parser's shape; an ordinary light-DOM child of the template
    // element would stay in-document instead.
    let tmpl = doc.append_element(Some(html), "template", Style::default(), None::<&str>);
    let frag = doc.allocate_template_fragment_root(tmpl);
    let ghost = doc.append_element(Some(frag), "body", Style::default(), None::<&str>);
    // Real body under html (should be picked)
    let real = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
    (doc, ghost, real)
}

/// Nest `count` divs under `parent`.
fn nest_divs(doc: &mut Document, parent: usize, count: usize) {
    let mut parent = parent;
    for _ in 0..count {
        parent = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
    }
}

#[test]
fn layout_find_body_skips_body_inside_template() {
    let (mut doc, _ghost, real) = document_with_template_ghost_body();
    doc.mark_in_document_flags();

    assert_eq!(
        layout_find_body(&doc),
        Some(real),
        "find_body must skip inert body inside <template> and select the real body"
    );
}

#[test]
fn paint_root_skips_body_inside_template() {
    let (mut doc, _ghost, real) = document_with_template_ghost_body();
    doc.mark_in_document_flags();

    assert_eq!(find_paint_root(&doc), Some(real));
}

#[test]
fn layout_depth_ignores_deep_nesting_inside_a_template_body() {
    let (mut doc, ghost, _real) = document_with_template_ghost_body();
    nest_divs(&mut doc, ghost, MAX_LAYOUT_DEPTH + 1);
    doc.mark_in_document_flags();

    assert!(validate_layout_depth(&doc).is_ok());
}

#[test]
fn layout_depth_counts_from_the_real_body() {
    let (mut doc, _ghost, real) = document_with_template_ghost_body();
    nest_divs(&mut doc, real, MAX_LAYOUT_DEPTH);
    doc.mark_in_document_flags();

    assert!(matches!(
        validate_layout_depth(&doc),
        Err(LayoutError::TreeDepthLimitExceeded { limit, actual })
            if limit == MAX_LAYOUT_DEPTH && actual == MAX_LAYOUT_DEPTH + 1
    ));
}
