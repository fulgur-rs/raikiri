//! Shared fixtures for the inline path's unit tests.

use super::font::{BundledFace, bundled_collection};
use crate::Document;
use raikiri_style::{CascadeResult, build_rule_tree, cascade};
use shodo::font::FontCollection;
use shodo::limits::Limits;

pub(crate) const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A cascaded document with one block under `<html><body>`.
pub(crate) struct Fixture {
    pub(crate) doc: Document,
    pub(crate) cascade: CascadeResult,
    pub(crate) root: usize,
}

/// Build `<div style="display:block;font-family:Ahem;font-size:10px;{extra}">`,
/// let `build` add its children, and cascade without a document font collection.
pub(crate) fn block_fixture(
    extra_style: &str,
    build: impl FnOnce(&mut Document, usize),
) -> Fixture {
    block_fixture_inner(extra_style, build, None)
}

/// [`block_fixture`] after installing the Ahem font collection and before cascading.
pub(crate) fn block_fixture_with_ahem(
    extra_style: &str,
    build: impl FnOnce(&mut Document, usize),
) -> Fixture {
    block_fixture_inner(extra_style, build, Some(ahem_fonts()))
}

/// [`block_fixture`] after installing the document's font collection and before cascading.
pub(crate) fn block_fixture_with_font_collection(
    extra_style: &str,
    build: impl FnOnce(&mut Document, usize),
    fonts: FontCollection,
) -> Fixture {
    block_fixture_inner(extra_style, build, Some(fonts))
}

fn block_fixture_inner(
    extra_style: &str,
    build: impl FnOnce(&mut Document, usize),
    fonts: Option<FontCollection>,
) -> Fixture {
    let mut doc = Document::new();
    // No UA sheet is applied here, so the ancestors are made block containers
    // explicitly (the initial `display` is `inline`).
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let style = format!("display:block;font-family:Ahem;font-size:10px;{extra_style}");
    let root = doc.append_element(Some(body), "div", Some(style.as_str()));
    build(&mut doc, root);
    doc.mark_in_document_flags();
    if let Some(fonts) = fonts {
        doc.set_font_collection(fonts);
    }
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    crate::layout::apply_computed_to_style(&mut doc, &cascade).expect("bridge fixture");
    Fixture { doc, cascade, root }
}

/// [`block_fixture`] with an author style sheet: `sheet` is the text of a
/// `<style>` element ahead of the body.
pub(crate) fn sheet_fixture(
    sheet: &str,
    extra_style: &str,
    build: impl FnOnce(&mut Document, usize),
) -> Fixture {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let head = doc.append_element(Some(html), "head", Some("display:none"));
    let style_element = doc.append_element(Some(head), "style", None::<&str>);
    doc.append_text(style_element, sheet);
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let style = format!("display:block;font-family:Ahem;font-size:10px;{extra_style}");
    let root = doc.append_element(Some(body), "div", Some(style.as_str()));
    build(&mut doc, root);
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    crate::layout::apply_computed_to_style(&mut doc, &cascade).expect("bridge fixture");
    Fixture { doc, cascade, root }
}

/// `<span style="{style}">` under `parent`.
pub(crate) fn span(doc: &mut Document, parent: usize, style: &str) -> usize {
    doc.append_element(Some(parent), "span", Some(style))
}

/// A shared layer holding only Ahem: every glyph is exactly one em wide.
pub(crate) fn ahem_fonts() -> FontCollection {
    bundled_collection(
        &Limits::default(),
        vec![BundledFace {
            family: "Ahem".to_owned(),
            bytes: AHEM.to_vec(),
        }],
        false,
    )
    .expect("bundled Ahem")
}
