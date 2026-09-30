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
