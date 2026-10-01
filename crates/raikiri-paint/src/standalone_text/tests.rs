use crate::text::{measure_margin_text, measure_margin_text_advance, measure_margin_text_height};
use raikiri_dom::Document;
use shodo::limits::Limits;

const FONT_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace"
);

fn engine_document() -> Document {
    let mut doc = Document::new();
    let collection = raikiri_dom::build_wpt_font_collection(std::path::Path::new(FONT_DIR))
        .expect("the Ahem layer");
    doc.enable_inline_formatting(collection, Limits::default());
    doc
}

#[test]
fn measurements_use_the_document_font_when_the_engine_is_on() {
    let doc = engine_document();
    assert_eq!(measure_margin_text(Some(&doc), "ab ", 10.0, "Ahem"), 20.0);
    assert_eq!(
        measure_margin_text_advance(Some(&doc), "ab ", 10.0, "Ahem"),
        30.0
    );
    assert_eq!(
        measure_margin_text_height(Some(&doc), "abc", 10.0, "Ahem"),
        10.0
    );
    assert_eq!(doc.standalone_text_calls(), 3);
}

#[test]
fn a_document_without_the_engine_measures_like_no_document() {
    // A pin: `shape` returns `None` for a document without the engine, so the
    // old path measures. It does not show that the old path ignores `document`.
    let off = Document::new();
    for content in ["abc", "ab ", ""] {
        assert_eq!(
            measure_margin_text(Some(&off), content, 16.0, "serif"),
            measure_margin_text(None, content, 16.0, "serif")
        );
        assert_eq!(
            measure_margin_text_advance(Some(&off), content, 16.0, "serif"),
            measure_margin_text_advance(None, content, 16.0, "serif")
        );
        assert_eq!(
            measure_margin_text_height(Some(&off), content, 16.0, "serif"),
            measure_margin_text_height(None, content, 16.0, "serif")
        );
    }
    assert_eq!(off.standalone_text_calls(), 0);
}

#[test]
fn what_the_engine_declines_is_measured_by_the_old_path() {
    let doc = engine_document();
    let hebrew = "ab \u{5d0}";
    assert_eq!(
        measure_margin_text(Some(&doc), hebrew, 16.0, "serif"),
        measure_margin_text(None, hebrew, 16.0, "serif")
    );
    assert_eq!(measure_margin_text(Some(&doc), "", 10.0, "Ahem"), 0.0);
    assert_eq!(doc.standalone_text_calls(), 0);
}

#[test]
fn a_family_list_string_is_split_and_unquoted() {
    let doc = engine_document();
    // Callers pass one family name, so the split is defensive. `Ahem` is the
    // second family here, in quotes.
    assert_eq!(
        measure_margin_text(Some(&doc), "abc", 10.0, "No Such, 'Ahem'"),
        30.0
    );
}

#[test]
fn an_unusable_font_size_falls_back_to_sixteen_like_before() {
    let doc = engine_document();
    // Ahem is one em per glyph, so the width follows the size actually used.
    assert_eq!(
        measure_margin_text(Some(&doc), "abc", f32::NAN, "Ahem"),
        48.0
    );
    assert_eq!(measure_margin_text(Some(&doc), "abc", 0.0, "Ahem"), 48.0);
    assert_eq!(
        measure_margin_text(Some(&doc), "abc", f32::INFINITY, "Ahem"),
        48.0
    );
}
