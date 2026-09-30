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
