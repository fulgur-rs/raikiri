//! Fixtures shared by the layout tests that need Ahem and a full page pass.

use crate::Document;
use crate::layout::ifc::test_support::{Fixture, block_fixture};
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;

thread_local! {
    pub(crate) static TEXT_LINE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static TABLE_CELL_LOOKUP_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static IFC_ATOMIC_OUTPUT_LOOKUP_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static IFC_ATOMIC_VISIBILITY_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static IFC_CANDIDATE_BOX_MEMBERSHIP_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static IFC_ROOT_BOX_MEMBERSHIP_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A `<div>` of Ahem text under `html > body`, with a fixed 10px line height.
///
/// A `\n` in `text` becomes a `<br>`. `css` is appended to the root's style.
pub(crate) fn ahem_paragraph(text: &str, css: &str) -> (Document, CascadeResult, usize) {
    let Fixture { doc, cascade, root } =
        block_fixture(&format!("line-height:10px;{css}"), |doc, root| {
            for (index, piece) in text.split('\n').enumerate() {
                if index > 0 {
                    doc.append_element(Some(root), "br", Some("display:inline"));
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
    let origin = doc.layout_origin_of(node).unwrap_or(taffy::Point::ZERO);
    let size = doc.nodes[node].unrounded_layout.size;
    (origin.x, origin.y, size.width, size.height)
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

/// `layout` run in a session over `doc` whose styles are bridged from a fresh
/// cascade of the document.
pub(crate) fn with_bridged_session<R>(
    doc: &mut Document,
    layout: impl FnOnce(&mut crate::layout::context::LayoutSession<'_>) -> R,
) -> R {
    let cascade = cascaded(doc);
    let mut session = crate::layout::context::LayoutSession::bridged(doc, &cascade);
    layout(&mut session)
}

/// `layout` run in a session over `doc` as an earlier bridge or layout left
/// it: the node fields a test patched and the results layout computed are
/// kept, as bridging again would rebuild them.
pub(crate) fn with_resumed_session<R>(
    doc: &mut Document,
    layout: impl FnOnce(&mut crate::layout::context::LayoutSession<'_>) -> R,
) -> R {
    let cascade = cascaded(doc);
    let mut session = crate::layout::context::LayoutSession::resumed(doc, &cascade);
    layout(&mut session)
}

/// `read` applied to a view of `values` under the pass overrides recorded for
/// box `key` of `doc`.
pub(crate) fn with_pass_view<R>(
    doc: &Document,
    key: crate::node::arena::BoxKey,
    values: &raikiri_style::ComputedValues,
    read: impl FnOnce(&raikiri_layout_style::LayoutStyleView<'_>) -> R,
) -> R {
    let prepared = raikiri_layout_style::PreparedStyle::prepare(
        values,
        raikiri_layout_style::StyleMetadata::default(),
    );
    let overrides: Vec<_> = doc
        .box_style_data
        .get(key)
        .expect("the box has style data")
        .pass
        .iter()
        .map(raikiri_layout_style::OwnedPropertyOverride::as_borrowed)
        .collect();
    read(&raikiri_layout_style::LayoutStyleView::new(
        &prepared,
        raikiri_layout_style::ViewContext::default(),
        &[],
        &overrides,
    ))
}

/// The declarations of `style` as the inline style of an element, for
/// fixtures that describe a box by the Taffy style it should have.
///
/// `display`, `box-sizing` and `position` are always written, as the Taffy
/// defaults differ from what CSS assumes; the other properties only when they
/// differ from the default style.
pub(crate) fn style_css(style: &crate::LayoutStyle) -> String {
    use taffy::{AlignContent, CompactLength, Display, Overflow};
    fn length(raw: CompactLength) -> Option<String> {
        match raw.tag() {
            CompactLength::LENGTH_TAG => Some(format!("{}px", raw.value())),
            CompactLength::PERCENT_TAG => Some(format!("{}%", (raw.value() * 1e6).round() / 1e4)),
            CompactLength::AUTO_TAG => Some("auto".to_owned()),
            CompactLength::MIN_CONTENT_TAG => Some("min-content".to_owned()),
            CompactLength::MAX_CONTENT_TAG => Some("max-content".to_owned()),
            CompactLength::FIT_CONTENT_KEYWORD_TAG => Some("fit-content".to_owned()),
            CompactLength::FIT_CONTENT_PX_TAG => Some(format!("fit-content({}px)", raw.value())),
            CompactLength::FIT_CONTENT_PERCENT_TAG => Some(format!(
                "fit-content({}%)",
                (raw.value() * 1e6).round() / 1e4
            )),
            tag => panic!("no CSS for the length tag {tag}"),
        }
    }
    let default = crate::LayoutStyle::default();
    let mut css = Vec::new();
    let mut declare = |name: &str, value: Option<String>| {
        if let Some(value) = value {
            css.push(format!("{name}:{value}"));
        }
    };
    declare(
        "display",
        Some(
            match style.display {
                Display::Block => "block",
                Display::FlowRoot => "flow-root",
                Display::Flex => "flex",
                Display::Grid => "grid",
                Display::None => "none",
            }
            .to_owned(),
        ),
    );
    declare(
        "box-sizing",
        Some(
            match style.box_sizing {
                taffy::BoxSizing::ContentBox => "content-box",
                taffy::BoxSizing::BorderBox => "border-box",
            }
            .to_owned(),
        ),
    );
    declare(
        "position",
        Some(
            match style.position {
                taffy::Position::Static => "static",
                taffy::Position::Relative => "relative",
                taffy::Position::Absolute => "absolute",
                taffy::Position::Fixed => "fixed",
                taffy::Position::Sticky => "sticky",
            }
            .to_owned(),
        ),
    );
    fn changed(raw: CompactLength, default: CompactLength) -> Option<String> {
        (raw != default).then(|| length(raw)).flatten()
    }
    declare(
        "width",
        changed(style.size.width.into_raw(), default.size.width.into_raw()),
    );
    declare(
        "height",
        changed(style.size.height.into_raw(), default.size.height.into_raw()),
    );
    declare(
        "min-width",
        changed(
            style.min_size.width.into_raw(),
            default.min_size.width.into_raw(),
        ),
    );
    declare(
        "min-height",
        changed(
            style.min_size.height.into_raw(),
            default.min_size.height.into_raw(),
        ),
    );
    declare(
        "max-width",
        changed(
            style.max_size.width.into_raw(),
            default.max_size.width.into_raw(),
        ),
    );
    declare(
        "max-height",
        changed(
            style.max_size.height.into_raw(),
            default.max_size.height.into_raw(),
        ),
    );
    for (side, margin, padding, border, inset) in [
        (
            "top",
            (style.margin.top.into_raw(), default.margin.top.into_raw()),
            (style.padding.top.into_raw(), default.padding.top.into_raw()),
            (style.border.top.into_raw(), default.border.top.into_raw()),
            (style.inset.top.into_raw(), default.inset.top.into_raw()),
        ),
        (
            "right",
            (
                style.margin.right.into_raw(),
                default.margin.right.into_raw(),
            ),
            (
                style.padding.right.into_raw(),
                default.padding.right.into_raw(),
            ),
            (
                style.border.right.into_raw(),
                default.border.right.into_raw(),
            ),
            (style.inset.right.into_raw(), default.inset.right.into_raw()),
        ),
        (
            "bottom",
            (
                style.margin.bottom.into_raw(),
                default.margin.bottom.into_raw(),
            ),
            (
                style.padding.bottom.into_raw(),
                default.padding.bottom.into_raw(),
            ),
            (
                style.border.bottom.into_raw(),
                default.border.bottom.into_raw(),
            ),
            (
                style.inset.bottom.into_raw(),
                default.inset.bottom.into_raw(),
            ),
        ),
        (
            "left",
            (style.margin.left.into_raw(), default.margin.left.into_raw()),
            (
                style.padding.left.into_raw(),
                default.padding.left.into_raw(),
            ),
            (style.border.left.into_raw(), default.border.left.into_raw()),
            (style.inset.left.into_raw(), default.inset.left.into_raw()),
        ),
    ] {
        declare(&format!("margin-{side}"), changed(margin.0, margin.1));
        declare(&format!("padding-{side}"), changed(padding.0, padding.1));
        if border.0 != border.1 {
            declare(&format!("border-{side}-width"), changed(border.0, border.1));
            declare(&format!("border-{side}-style"), Some("solid".to_owned()));
        }
        declare(side, changed(inset.0, inset.1));
    }
    for (name, overflow) in [
        ("overflow-x", style.overflow.x),
        ("overflow-y", style.overflow.y),
    ] {
        declare(
            name,
            match overflow {
                Overflow::Visible => None,
                Overflow::Clip => Some("clip".to_owned()),
                Overflow::Hidden => Some("hidden".to_owned()),
                Overflow::Scroll => Some("scroll".to_owned()),
            },
        );
    }
    declare(
        "float",
        match style.float {
            taffy::Float::None => None,
            taffy::Float::Left => Some("left".to_owned()),
            taffy::Float::Right => Some("right".to_owned()),
        },
    );
    declare(
        "clear",
        match style.clear {
            taffy::Clear::None => None,
            taffy::Clear::Left => Some("left".to_owned()),
            taffy::Clear::Right => Some("right".to_owned()),
            taffy::Clear::Both => Some("both".to_owned()),
        },
    );
    declare(
        "direction",
        (style.direction != default.direction).then(|| "rtl".to_owned()),
    );
    declare(
        "aspect-ratio",
        style.aspect_ratio.map(|ratio| ratio.to_string()),
    );
    if style.align_content != default.align_content {
        let keyword = [
            (AlignContent::START, "start"),
            (AlignContent::END, "end"),
            (AlignContent::CENTER, "center"),
            (AlignContent::FLEX_START, "flex-start"),
            (AlignContent::FLEX_END, "flex-end"),
            (AlignContent::STRETCH, "stretch"),
            (AlignContent::SPACE_BETWEEN, "space-between"),
            (AlignContent::SPACE_AROUND, "space-around"),
            (AlignContent::SPACE_EVENLY, "space-evenly"),
        ]
        .into_iter()
        .find_map(|(candidate, name)| (candidate == style.align_content).then_some(name))
        .expect("an align-content keyword");
        declare("align-content", Some(keyword.to_owned()));
    }
    css.join(";")
}

/// Appends `declarations` to the inline style of element `node`; they override
/// the declarations they repeat.
pub(crate) fn restyle(doc: &mut Document, node: usize, declarations: &str) {
    let crate::node::NodeData::Element(element) = &doc.nodes[node].data else {
        panic!("restyle needs an element");
    };
    let mut css = element
        .inline_style
        .as_deref()
        .unwrap_or_default()
        .to_owned();
    css.push(';');
    css.push_str(declarations);
    doc.set_element_inline_style(node, Some(css.into()));
}

/// The physical style input of `node` once `cascade` is bridged into `doc`.
///
/// The bridge runs in a session of its own, so the style reflects the
/// document as it is now, not as an earlier layout left it.
pub(crate) fn bridged_style(
    doc: &mut Document,
    cascade: &CascadeResult,
    node: usize,
) -> crate::LayoutStyle {
    let session = crate::layout::context::LayoutSession::bridged(doc, cascade);
    crate::taffy_impl::block_flow::style_input::physical(&session, node)
}

/// The display of `node` once `cascade` is bridged into `doc`, in the terms
/// layout dispatches on (`flow-root`, `grid`, ...).
pub(crate) fn bridged_display(
    doc: &mut Document,
    cascade: &CascadeResult,
    node: usize,
) -> taffy::Display {
    let session = crate::layout::context::LayoutSession::bridged(doc, cascade);
    session.nodes[node].taffy_display()
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let wrapper = doc.append_element(
        Some(body),
        "div",
        Some(&format!("{AHEM_FAMILY_CSS}{wrapper_css}")),
    );
    let inner = doc.append_element(Some(wrapper), "div", Some("display:block"));
    let root = doc.append_element(
        Some(inner),
        "div",
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let wrapper = doc.append_element(
        Some(body),
        "div",
        Some(&format!(
            "display:block;width:100px;{AHEM_FAMILY_CSS}line-height:10px"
        )),
    );
    let floats: Vec<usize> = float_css
        .iter()
        .map(|css| doc.append_element(Some(wrapper), "div", Some(&format!("display:block;{css}"))))
        .collect();
    let root = doc.append_element(
        Some(wrapper),
        "div",
        Some(&format!("display:block;{root_css}")),
    );
    doc.append_text(root, text);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, floats, root)
}

/// `ahem_paragraph_beside_float` with the text inside a `span` styled by
/// `span_css`, and the author style sheet `sheet`. Returns
/// `(doc, cascade, root)`.
pub(crate) fn ahem_span_beside_float(
    text: &str,
    float_css: &str,
    root_css: &str,
    span_css: &str,
    sheet: &str,
) -> (Document, CascadeResult, usize) {
    let (mut doc, _, _, root) = ahem_paragraph_beside_floats("", &[float_css], root_css);
    let style_element = doc.append_element(Some(root), "style", Some("display:none"));
    doc.append_text(style_element, sheet);
    let span = doc.append_element(
        Some(root),
        "span",
        Some(&format!("display:inline;{span_css}")),
    );
    doc.append_text(span, text);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    (doc, cascade, root)
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let root = doc.append_element(
        Some(body),
        "div",
        Some(&format!(
            "display:block;width:100px;{AHEM_FAMILY_CSS}line-height:10px;{root_css}"
        )),
    );
    doc.append_text(root, before);
    let float = doc.append_element(
        Some(root),
        "div",
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let root = doc.append_element(
        Some(body),
        "div",
        Some(&format!(
            "display:block;width:100px;{AHEM_FAMILY_CSS}line-height:10px;{root_css}"
        )),
    );
    doc.append_text(root, before);
    let atomic = doc.append_element(
        Some(root),
        "span",
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    (doc, body)
}

/// Mark the flags of `doc` and cascade it.
pub(crate) fn cascaded(doc: &Document) -> CascadeResult {
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
        Some(&format!("display:block;{AHEM_FAMILY_CSS}{parent_css}")),
    );
    let root = doc.append_element(
        Some(parent),
        "div",
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
        Some(&format!(
            "display:flex;align-items:flex-start;{AHEM_FAMILY_CSS}line-height:10px"
        )),
    );
    let item = doc.append_element(Some(flex), "span", None::<&str>);
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
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some(&format!("display:block;{AHEM_FAMILY_CSS}line-height:10px")),
    );
    doc.append_text(body, text);
    let div = doc.append_element(Some(body), "div", Some("display:block"));
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
        doc.append_element(Some(root), "span", Some(atomic_css));
    })
}

/// A paragraph whose only content is `count` `<br>` elements.
pub(crate) fn paragraph_with_only_br(count: usize) -> (Document, CascadeResult, usize) {
    ahem_root_with("", |doc, root| {
        for _ in 0..count {
            doc.append_element(Some(root), "br", Some("display:inline"));
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
        Some(&format!(
            "display:table;{AHEM_FAMILY_CSS}line-height:10px;{table_css}"
        )),
    );
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let cell = doc.append_element(
        Some(row),
        "div",
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
        Some(&format!(
            "display:table;{AHEM_FAMILY_CSS}line-height:10px;{table_css}"
        )),
    );
    doc.append_text(table, text);
    doc.mark_in_document_flags();
    let cascade = cascaded(&doc);
    (doc, cascade, table)
}
