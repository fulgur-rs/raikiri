use super::*;
use crate::Document;
use crate::layout::ifc::test_support::ahem_fonts;
use crate::layout::test_support::with_ahem;

#[test]
fn debug_reports_fragment_box_placement_count() {
    let lines = IfcLines {
        width: 100.0,
        lines: std::sync::Arc::new(Vec::new()),
        height: 0.0,
        beside_floats: false,
        escaping_margin: taffy::CollapsibleMarginSet::ZERO,
        block_line_starts: Vec::new(),
        shifts: Vec::new(),
        fragment_box_placements: vec![IfcBoxFragment {
            node_id: 1,
            fragmentainer: 0,
            rect: crate::fragment::FragmentRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 20.0,
            },
        }],
        fragmentainer_line_ranges: None,
    };

    assert!(format!("{lines:?}").contains("fragment_box_placements: 1"));
}

#[test]
fn a_document_opts_in_explicitly() {
    let mut doc = Document::new();
    assert!(!doc.has_font_collection());
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    assert!(doc.has_font_collection());
}

#[test]
fn a_cloned_document_keeps_its_fonts() {
    let mut doc = Document::new();
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    assert!(doc.clone().has_font_collection());
}

#[test]
fn with_state_puts_the_state_back() {
    let mut doc = Document::new();
    doc.set_font_collection_with_limits(ahem_fonts(), Limits::default());
    let seen = with_state(&mut doc, |state| state.limits.clone());
    assert!(seen.is_some());
    assert!(doc.has_font_collection(), "the state must be restored");
}

#[test]
fn with_state_does_nothing_without_fonts() {
    let mut doc = Document::new();
    assert!(with_state(&mut doc, |_| ()).is_none());
    assert!(!doc.has_font_collection());
}

#[test]
fn a_cloned_document_shares_the_lines_of_its_ifc_roots() {
    use crate::layout::test_support::{ahem_paragraph, ifc_ahem_fonts, page_box_800x600};
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:40px");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
        .expect("layout");
    let copy = doc.clone();
    let a = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .expect("lines");
    let b = copy.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .expect("lines");
    assert!(std::sync::Arc::ptr_eq(&a.lines, &b.lines));
}

#[test]
fn a_rebreak_that_skips_a_root_keeps_the_same_lines() {
    use crate::layout::test_support::{
        ahem_paragraph_with_atomic, ifc_ahem_fonts, page_box_800x600,
    };
    // A root with a box of its own keeps the lines its box was placed with.
    let (mut doc, cascade, _atomic, root) =
        ahem_paragraph_with_atomic("aa ", "width:10px;height:10px", " bb", "");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600())
        .expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    let before = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .map(|l| std::sync::Arc::clone(&l.lines))
        .expect("lines");
    crate::layout::relayout_text_for_width(with_ahem(&mut doc), &cascade, 30.0);
    let after = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .expect("lines");
    assert!(std::sync::Arc::ptr_eq(&before, &after.lines));
}
