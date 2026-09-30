//! Fixtures shared by the layout tests that need Ahem and a full page pass.

use crate::Document;
use crate::layout::ifc::test_support::{AHEM, Fixture, block_fixture};
use parley::FontContext;
use parley::fontique::Blob;
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;
use std::sync::Arc;

/// A `<div>` of Ahem text under `html > body`, with a fixed 10px line height.
///
/// A `\n` in `text` becomes a `<br>`. `css` is appended to the root's style.
pub(crate) fn ahem_paragraph(text: &str, css: &str) -> (Document, CascadeResult, usize) {
    let Fixture { doc, cascade, root } =
        block_fixture(&format!("line-height:10px;{css}"), |doc, root| {
            for (index, piece) in text.split('\n').enumerate() {
                if index > 0 {
                    doc.append_element(
                        Some(root),
                        "br",
                        taffy::Style::default(),
                        Some("display:inline"),
                    );
                }
                if !piece.is_empty() {
                    doc.append_text(root, piece);
                }
            }
        });
    (doc, cascade, root)
}

/// Font declarations shared by the Ahem paragraph fixtures.
const AHEM_FAMILY_CSS: &str = "font-family:Ahem;font-size:10px;";

/// A parley font context holding only Ahem.
pub(crate) fn ahem_font_context() -> FontContext {
    let mut ctx = FontContext::new();
    let registered = ctx
        .collection
        .register_fonts(Blob::new(Arc::new(AHEM.to_vec()) as _), None);
    assert!(!registered.is_empty(), "Ahem must register");
    ctx
}

/// An 800x600 page with no margins.
pub(crate) fn page_box_800x600() -> PageBox {
    let mut page = PageBox::new();
    page.width = 800.0;
    page.height = 600.0;
    page
}

/// The Ahem font layer for the shodo engine.
pub(crate) fn ifc_ahem_fonts() -> shodo::font::FontCollection {
    crate::layout::ifc::test_support::ahem_fonts()
}

/// `html > body > div(wrapper_css) > div(display:block) > div(root)`: the
/// paragraph root sits in a block wrapper, so the wrapper may itself be an
/// inline-block or a float. Returns `(doc, cascade, wrapper, root)`.
pub(crate) fn ahem_paragraph_in_block_wrapper(
    text: &str,
    wrapper_css: &str,
) -> (Document, CascadeResult, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        taffy::Style::default(),
        Some("display:block"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("display:block"),
    );
    let wrapper = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!("{AHEM_FAMILY_CSS}{wrapper_css}")),
    );
    let inner = doc.append_element(
        Some(wrapper),
        "div",
        taffy::Style::default(),
        Some("display:block"),
    );
    let root = doc.append_element(
        Some(inner),
        "div",
        taffy::Style::default(),
        Some(&format!("display:block;{AHEM_FAMILY_CSS}line-height:10px")),
    );
    doc.append_text(root, text);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, wrapper, root)
}

/// `html > body > div(width:100px) > [float divs.., root div]`. The floats are
/// siblings placed before the root. Returns `(doc, cascade, floats, root)`.
pub(crate) fn ahem_paragraph_beside_floats(
    text: &str,
    float_css: &[&str],
    root_css: &str,
) -> (Document, CascadeResult, Vec<usize>, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        taffy::Style::default(),
        Some("display:block"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("display:block"),
    );
    let wrapper = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!(
            "display:block;width:100px;{AHEM_FAMILY_CSS}line-height:10px"
        )),
    );
    let floats: Vec<usize> = float_css
        .iter()
        .map(|css| {
            doc.append_element(
                Some(wrapper),
                "div",
                taffy::Style::default(),
                Some(&format!("display:block;{css}")),
            )
        })
        .collect();
    let root = doc.append_element(
        Some(wrapper),
        "div",
        taffy::Style::default(),
        Some(&format!("display:block;{root_css}")),
    );
    doc.append_text(root, text);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, floats, root)
}

/// The one-float form of [`ahem_paragraph_beside_floats`].
pub(crate) fn ahem_paragraph_beside_float(
    text: &str,
    float_css: &str,
    root_css: &str,
) -> (Document, CascadeResult, usize, usize) {
    let (doc, cascade, floats, root) = ahem_paragraph_beside_floats(text, &[float_css], root_css);
    (doc, cascade, floats[0], root)
}

/// Offset of the first glyph of a line from the content-box start.
pub(crate) fn line_start_x(line: &shodo::Line) -> Option<f32> {
    line.fragments().find_map(|fragment| match fragment {
        shodo::Fragment::GlyphRun(run) => run.glyph_origin(0).map(|(x, _)| x),
        _ => None,
    })
}

/// Text of a line without the trailing break spaces and without the U+FFFC
/// placeholders shodo puts in the text for float anchors and atomics.
pub(crate) fn line_text(line: &shodo::Line) -> String {
    line.text()[line.text_range()]
        .replace('\u{FFFC}', "")
        .trim_end()
        .to_owned()
}

/// `html > body > div(root)` whose content is `before`, a float div, `after`.
/// Returns `(doc, cascade, float, root)`.
pub(crate) fn ahem_paragraph_with_float(
    before: &str,
    float_css: &str,
    after: &str,
    root_css: &str,
) -> (Document, CascadeResult, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        taffy::Style::default(),
        Some("display:block"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("display:block"),
    );
    let root = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!(
            "display:block;width:100px;{AHEM_FAMILY_CSS}line-height:10px;{root_css}"
        )),
    );
    doc.append_text(root, before);
    let float = doc.append_element(
        Some(root),
        "div",
        taffy::Style::default(),
        Some(&format!("display:block;{float_css}")),
    );
    doc.append_text(root, after);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, float, root)
}

/// `html > body > div(root)` whose content is `before`, an inline-block span,
/// `after`. Returns `(doc, cascade, atomic, root)`.
pub(crate) fn ahem_paragraph_with_atomic(
    before: &str,
    atomic_css: &str,
    after: &str,
    root_css: &str,
) -> (Document, CascadeResult, usize, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        taffy::Style::default(),
        Some("display:block"),
    );
    let body = doc.append_element(
        Some(html),
        "body",
        taffy::Style::default(),
        Some("display:block"),
    );
    let root = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!(
            "display:block;width:100px;{AHEM_FAMILY_CSS}line-height:10px;{root_css}"
        )),
    );
    doc.append_text(root, before);
    let atomic = doc.append_element(
        Some(root),
        "span",
        taffy::Style::default(),
        Some(&format!("display:inline-block;{atomic_css}")),
    );
    doc.append_text(root, after);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, atomic, root)
}
