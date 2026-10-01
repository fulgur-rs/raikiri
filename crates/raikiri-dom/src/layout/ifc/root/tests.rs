use super::*;
use crate::Document;
use crate::layout::ifc::test_support::ahem_fonts;

#[test]
fn a_document_opts_in_explicitly() {
    let mut doc = Document::new();
    assert!(!doc.inline_formatting_enabled());
    doc.enable_inline_formatting(ahem_fonts(), Limits::default());
    assert!(doc.inline_formatting_enabled());
}

#[test]
fn a_cloned_document_keeps_the_switch() {
    let mut doc = Document::new();
    doc.enable_inline_formatting(ahem_fonts(), Limits::default());
    assert!(doc.clone().inline_formatting_enabled());
}

#[test]
fn with_state_puts_the_state_back() {
    let mut doc = Document::new();
    doc.enable_inline_formatting(ahem_fonts(), Limits::default());
    let seen = with_state(&mut doc, |state| state.limits.clone());
    assert!(seen.is_some());
    assert!(
        doc.inline_formatting_enabled(),
        "the state must be restored"
    );
}

#[test]
fn with_state_does_nothing_without_the_switch() {
    let mut doc = Document::new();
    assert!(with_state(&mut doc, |_| ()).is_none());
    assert!(!doc.inline_formatting_enabled());
}

#[test]
fn a_cloned_document_shares_the_lines_of_its_ifc_roots() {
    use crate::layout::test_support::{
        ahem_font_context, ahem_paragraph, ifc_ahem_fonts, page_box_800x600,
    };
    let (mut doc, cascade, root) = ahem_paragraph("aaaa bbbb cccc", "width:40px");
    doc.enable_inline_formatting(ifc_ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600(), ahem_font_context())
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
        ahem_font_context, ahem_paragraph_with_atomic, ifc_ahem_fonts, page_box_800x600,
    };
    // A root with a box of its own keeps the lines its box was placed with.
    let (mut doc, cascade, _atomic, root) =
        ahem_paragraph_with_atomic("aa ", "width:10px;height:10px", " bb", "");
    doc.enable_inline_formatting(ifc_ahem_fonts(), Limits::default());
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600(), ahem_font_context())
        .expect("layout");
    assert!(doc.nodes[root].is_ifc_root());
    let before = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .map(|l| std::sync::Arc::clone(&l.lines))
        .expect("lines");
    crate::layout::relayout_text_for_width(&mut doc, &cascade, 30.0, 30.0, ahem_font_context());
    let after = doc.nodes[root]
        .ifc
        .as_ref()
        .and_then(|r| r.lines.as_ref())
        .expect("lines");
    assert!(std::sync::Arc::ptr_eq(&before, &after.lines));
}
