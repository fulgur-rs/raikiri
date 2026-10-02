//! Fixtures shared by the layout tests that need Ahem and a full page pass.

use crate::Document;
use crate::layout::ifc::test_support::{Fixture, block_fixture};
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;

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

/// `block_fixture` with the paragraph's line height set; `build` fills the
/// root. Returns the document, its cascade and the root.
pub(crate) fn ahem_paragraph_with(
    css: &str,
    build: impl FnOnce(&mut Document, usize),
) -> (Document, CascadeResult, usize) {
    let Fixture { doc, cascade, root } = block_fixture(&format!("line-height:10px;{css}"), build);
    (doc, cascade, root)
}

/// The border box of `node` in page coordinates, accumulated from the layout
/// locations of its ancestors.
pub(crate) fn absolute_rect(doc: &Document, node: usize) -> (f32, f32, f32, f32) {
    let (mut x, mut y) = (0.0, 0.0);
    let mut current = Some(node);
    while let Some(id) = current {
        let layout = doc.nodes[id].unrounded_layout;
        x += layout.location.x;
        y += layout.location.y;
        current = doc.parent_of(id);
    }
    let size = doc.nodes[node].unrounded_layout.size;
    (x, y, size.width, size.height)
}

/// Font declarations shared by the Ahem paragraph fixtures.
const AHEM_FAMILY_CSS: &str = "font-family:Ahem;font-size:10px;";

/// `doc` with the Ahem font layer, unless it already has fonts.
pub(crate) fn with_ahem(doc: &mut Document) -> &mut Document {
    if !doc.has_font_collection() {
        doc.set_font_collection(ifc_ahem_fonts());
    }
    doc
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

/// `html > body` with `display:block` on both, ready for a test's own content.
/// Returns `(doc, body)`.
fn html_body() -> (Document, usize) {
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
    (doc, body)
}

/// Mark the flags of `doc` and cascade it.
fn cascaded(doc: &Document) -> CascadeResult {
    let rules = raikiri_style::build_rule_tree(doc);
    raikiri_style::cascade(doc, &rules).expect("cascade")
}

/// `html > body > div(parent_css) > div(css) > text`: an Ahem paragraph with a
/// 10px line height under a parent of any display. The inner div is a block
/// unless `css` says otherwise. Returns the inner div.
pub(crate) fn ahem_paragraph_in(
    parent_css: &str,
    css: &str,
    text: &str,
) -> (Document, CascadeResult, usize) {
    let (mut doc, body) = html_body();
    let parent = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!("display:block;{AHEM_FAMILY_CSS}{parent_css}")),
    );
    let root = doc.append_element(
        Some(parent),
        "div",
        taffy::Style::default(),
        Some(&format!("display:block;line-height:10px;{css}")),
    );
    doc.append_text(root, text);
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    (doc, cascade, root)
}

/// `html > body > div(display:flex) > span > text`: the span keeps its
/// initial `display:inline` and is a flex item only through taffy's
/// blockification. Returns the span.
pub(crate) fn flex_container_with_inline_item(text: &str) -> (Document, CascadeResult, usize) {
    let (mut doc, body) = html_body();
    let flex = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!(
            "display:flex;align-items:flex-start;{AHEM_FAMILY_CSS}line-height:10px"
        )),
    );
    let item = doc.append_element(Some(flex), "span", taffy::Style::default(), None::<&str>);
    doc.append_text(item, text);
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    (doc, cascade, item)
}

/// `html > body > [text, div(display:block) > inner]`: body holds inline text
/// and a block child. Returns `(doc, cascade, body, div)`.
pub(crate) fn body_with_text_and_block_child(
    text: &str,
    inner: &str,
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
        Some(&format!("display:block;{AHEM_FAMILY_CSS}line-height:10px")),
    );
    doc.append_text(body, text);
    let div = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some("display:block"),
    );
    doc.append_text(div, inner);
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    (doc, cascade, body, div)
}

/// `html > body > div(root)` whose content is built by `build`; the root is an
/// Ahem block with a 10px line height and `root_css`.
fn ahem_root_with(
    root_css: &str,
    build: impl FnOnce(&mut Document, usize),
) -> (Document, CascadeResult, usize) {
    let (mut doc, body) = html_body();
    let root = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!(
            "display:block;{AHEM_FAMILY_CSS}line-height:10px;{root_css}"
        )),
    );
    build(&mut doc, root);
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    (doc, cascade, root)
}

/// A paragraph whose only content is one inline-block span (`atomic_css`)
/// without text.
pub(crate) fn paragraph_with_only_atomic(atomic_css: &str) -> (Document, CascadeResult, usize) {
    ahem_root_with("", |doc, root| {
        doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some(atomic_css),
        );
    })
}

/// A paragraph whose only content is `count` `<br>` elements.
pub(crate) fn paragraph_with_only_br(count: usize) -> (Document, CascadeResult, usize) {
    ahem_root_with("", |doc, root| {
        for _ in 0..count {
            doc.append_element(
                Some(root),
                "br",
                taffy::Style::default(),
                Some("display:inline"),
            );
        }
    })
}

/// A paragraph whose only content is one empty inline span (`span_css`).
pub(crate) fn paragraph_with_only_an_empty_span(
    span_css: &str,
) -> (Document, CascadeResult, usize) {
    ahem_root_with("", |doc, root| {
        doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some(&format!("display:inline;{span_css}")),
        );
    })
}

/// `html > body > div(root, width:200px) > ["x", span(inline-block, width) > text]`.
/// Returns `(doc, cascade, root, inline_block)`.
pub(crate) fn paragraph_with_inline_block(
    text: &str,
    width: f32,
) -> (Document, CascadeResult, usize, usize) {
    let mut inline_block = 0;
    let (doc, cascade, root) = ahem_root_with("width:200px", |doc, root| {
        doc.append_text(root, "x");
        inline_block = doc.append_element(
            Some(root),
            "span",
            taffy::Style::default(),
            Some(&format!("display:inline-block;width:{width}px")),
        );
        doc.append_text(inline_block, text);
    });
    (doc, cascade, root, inline_block)
}

/// `html > body > div(parent_css) > text`: Ahem text with a 10px line height
/// directly in a container of any display. Returns the container.
pub(crate) fn ahem_paragraph_in_text_only(
    parent_css: &str,
    text: &str,
) -> (Document, CascadeResult, usize) {
    let (mut doc, body) = html_body();
    let parent = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!(
            "display:block;{AHEM_FAMILY_CSS}line-height:10px;{parent_css}"
        )),
    );
    doc.append_text(parent, text);
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    (doc, cascade, parent)
}

/// `html > body > div(display:table; table_css) > div(display:table-row) >
/// div(display:table-cell; cell_css)`, Ahem 10px with a 10px line height;
/// `build` fills the cell. Returns the table, the row and the cell.
pub(crate) fn ahem_table(
    table_css: &str,
    cell_css: &str,
    build: impl FnOnce(&mut Document, usize),
) -> (Document, CascadeResult, [usize; 3]) {
    let (mut doc, body) = html_body();
    let table = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!(
            "display:table;{AHEM_FAMILY_CSS}line-height:10px;{table_css}"
        )),
    );
    let row = doc.append_element(
        Some(table),
        "div",
        taffy::Style::default(),
        Some("display:table-row"),
    );
    let cell = doc.append_element(
        Some(row),
        "div",
        taffy::Style::default(),
        Some(&format!("display:table-cell;{cell_css}")),
    );
    build(&mut doc, cell);
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    (doc, cascade, [table, row, cell])
}

/// `html > body > div(display:table; table_css)` holding only `text`, Ahem
/// 10px with a 10px line height. Returns the table.
pub(crate) fn ahem_table_with_only_text(
    table_css: &str,
    text: &str,
) -> (Document, CascadeResult, usize) {
    let (mut doc, body) = html_body();
    let table = doc.append_element(
        Some(body),
        "div",
        taffy::Style::default(),
        Some(&format!(
            "display:table;{AHEM_FAMILY_CSS}line-height:10px;{table_css}"
        )),
    );
    doc.append_text(table, text);
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    (doc, cascade, table)
}
