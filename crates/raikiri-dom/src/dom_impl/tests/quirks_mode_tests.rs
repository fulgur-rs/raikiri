use super::super::*;
use crate::document::Document;

/// A freshly-constructed `Document` (no parse involved) defaults to
/// `QuirksMode::NoQuirks`, matching both the type's own `#[default]` and
/// `StyleDom::quirks_mode`'s documented safe default for shell
/// implementations — the two must agree when nothing has called
/// `Document::set_quirks_mode` yet (e.g. raikiri-dom unit tests /
/// raikiri-paint hello-world setup that build a `Document` by hand).
#[test]
fn document_default_quirks_mode_is_no_quirks() {
    let doc = Document::new();
    assert_eq!(doc.quirks_mode(), QuirksMode::NoQuirks);
    assert_eq!(StyleDom::quirks_mode(&doc), StyleQuirksMode::NoQuirks);
}

/// All 3 `QuirksMode` variants round-trip through `Document::
/// set_quirks_mode` → `Document::quirks_mode` → `impl StyleDom for
/// Document`'s `quirks_mode()` override into the matching
/// `StyleQuirksMode` variant — this is the conversion
/// `RaikiriTreeSink::finish` (raikiri-html) relies on to carry
/// html5ever's parse-time quirks-mode detection all the way to
/// `raikiri-style`'s cascade.
#[test]
fn set_quirks_mode_round_trips_through_style_dom_for_all_variants() {
    for (native, style) in [
        (QuirksMode::NoQuirks, StyleQuirksMode::NoQuirks),
        (QuirksMode::LimitedQuirks, StyleQuirksMode::LimitedQuirks),
        (QuirksMode::Quirks, StyleQuirksMode::Quirks),
    ] {
        let mut doc = Document::new();
        doc.set_quirks_mode(native);
        assert_eq!(doc.quirks_mode(), native);
        assert_eq!(StyleDom::quirks_mode(&doc), style);
    }
}
