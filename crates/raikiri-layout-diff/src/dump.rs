//! The canonical text of a laid-out document.
//!
//! Two builds of the layout engine are compared by the text this module
//! writes for the same input, so the text has to hold everything the layout
//! decided and nothing that varies between runs of the same build. It has two
//! parts:
//!
//! - the page API a consumer such as fulgur reads: the page geometry, the
//!   root box, the fragments, the positioned glyph runs with their opacity
//!   groups, the paint order and the paint order for those runs, the
//!   page-margin boxes with the glyphs of their text, and the links of every
//!   page. The paint events of list-marker images and of images of `content`
//!   are written with the place the page gave them;
//! - the laid-out box of every node of each distinct laid-out document, read
//!   through [`crate::node_layout::NodeBox`], the style getters of the box,
//!   written by [`crate::style_view`], and the lines of every paragraph, with
//!   the pieces of the lines in a vertical writing mode, whose text the page
//!   API gives no runs for.
//!
//! Numbers are written by [`number`], which keeps every `f32` apart from every
//! other: the shortest decimal text that parses back to the same value, so
//! two floats print the same exactly when their bits are the same, and a NaN
//! carries its payload. A change of a single ulp, or of the sign of a zero,
//! shows in the text.
//!
//! Values that are plain data with a stable `Debug` form are written with it.
//! Handles into the layout's internals, such as fragments and the shaped font
//! faces of a run, are written field by field through their accessors, so a
//! change to what they hold internally does not show as a difference.

use std::fmt::Write as _;

use raikiri_html::{DocumentLayout, Fragment, MarginBox, Page, PaintEvent, PositionedGlyphRun};
use raikiri_traits::PaintRect;

use crate::node_layout::NodeBox;

/// Longest text of a text node the node listing quotes.
const QUOTED_TEXT_CHARS: usize = 24;

/// 64-bit FNV-1a, a stable hash for comparing dumps across processes.
pub(crate) fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// `value` as text that is equal for two floats exactly when their bits are.
///
/// Rust prints the shortest decimal text that reads back as the same `f32`,
/// which tells every finite value, infinity and signed zero apart. A NaN is
/// written with its bits, as NaNs with different payloads print the same.
pub(crate) fn number(value: f32) -> String {
    if value.is_nan() {
        format!("NaN:{:08x}", value.to_bits())
    } else {
        format!("{value:?}")
    }
}

/// `values` as a bracketed list of [`number`]s.
fn numbers(values: &[f32]) -> String {
    let items: Vec<String> = values.iter().map(|value| number(*value)).collect();
    format!("[{}]", items.join(" "))
}

fn rect(rect: &PaintRect) -> String {
    numbers(&[rect.x, rect.y, rect.width, rect.height])
}

/// Writes the canonical text of `layout`: its pages, then the boxes of the
/// laid-out documents behind them.
pub(crate) fn document(out: &mut String, layout: &DocumentLayout) {
    let _ = writeln!(out, "pages: {}", layout.page_count());
    let _ = writeln!(out, "warnings: {}", layout.warnings().len());
    for warning in layout.warnings() {
        let _ = writeln!(out, "  {warning:?}");
    }
    // Pages after a change of content width are laid out again, in a
    // document of their own; pages that share a document are listed once, by
    // the first of them.
    let mut firsts: Vec<Page<'_>> = Vec::new();
    for page in layout.pages() {
        self::page(out, &page);
        let document = page.paint_inputs().0;
        if !firsts
            .iter()
            .any(|seen| std::ptr::eq(seen.paint_inputs().0, document))
        {
            firsts.push(page);
        }
    }
    for (index, page) in firsts.iter().enumerate() {
        let _ = writeln!(
            out,
            "document {index} (first used by page {})",
            page.index()
        );
        nodes(out, page);
    }
}

fn page(out: &mut String, page: &Page<'_>) {
    let _ = writeln!(out, "page {} name={:?}", page.index(), page.name());
    let _ = writeln!(out, "geometry: {:?}", page.geometry());
    let _ = writeln!(out, "root_box: {:?}", page.root_box());
    let fragments: Vec<Fragment<'_>> = page.fragments().collect();
    let _ = writeln!(out, "fragments: {}", fragments.len());
    for fragment in &fragments {
        self::fragment(out, "fragment", fragment);
    }
    let clips: Vec<_> = page.overflow_clips().collect();
    let _ = writeln!(out, "overflow_clips: {}", clips.len());
    for clip in &clips {
        let _ = writeln!(out, "  {clip:?}");
    }
    let runs = page.text_runs();
    let _ = writeln!(out, "text_runs: {}", runs.len());
    for run in &runs {
        text_run(out, run);
        // The inline opacity group decides which line event draws the run.
        let _ = writeln!(
            out,
            "    opacity_group={:?}",
            page.text_run_opacity_group(run)
        );
    }
    let events = page.paint_order();
    let _ = writeln!(out, "paint_order: {}", events.len());
    for event in &events {
        paint_event(out, event, page);
    }
    // The order a consumer that paints the runs above follows: text lines and
    // the opacity groups of inline elements.
    let events = page.paint_order_for_text_runs(&runs);
    let _ = writeln!(out, "paint_order_for_text_runs: {}", events.len());
    for event in &events {
        paint_event(out, event, page);
    }
    let boxes = page.margin_boxes();
    let _ = writeln!(out, "margin_boxes: {}", boxes.len());
    for margin_box in &boxes {
        self::margin_box(out, margin_box);
    }
    let links: Vec<_> = page.links().collect();
    let _ = writeln!(out, "links: {}", links.len());
    for link in &links {
        let _ = writeln!(out, "{link:#?}");
    }
}

/// Writes a page-margin box: its fields, the glyph runs of its text, and the
/// shaped lines behind them. The box's `Debug` form counts the lines; text in
/// a vertical writing mode has no glyph runs, so its lines are listed with the
/// pieces they hold.
fn margin_box(out: &mut String, margin_box: &MarginBox) {
    let _ = writeln!(out, "{margin_box:#?}");
    for run in margin_box.text_runs() {
        text_run(out, &run);
    }
    let Some(text) = &margin_box.text else {
        return;
    };
    let _ = writeln!(out, "  shaped {:?}", text.shaped());
    for (index, line) in text.shaped().lines().iter().enumerate() {
        let mode = line.writing_mode();
        shaped_line(
            out,
            index,
            mode,
            line.block_offset(),
            line.metrics(),
            mode.is_vertical().then(|| line.fragments()),
        );
    }
}

/// Writes a shaped line: its writing mode, its offset and its metrics, then
/// the `pieces` it holds when the caller passes them. Callers pass the pieces
/// of lines in a vertical writing mode only: the runs of the page API carry
/// the placement of horizontal text, and vertical text has none.
fn shaped_line<P: std::fmt::Debug>(
    out: &mut String,
    index: usize,
    mode: impl std::fmt::Debug,
    block_offset: f32,
    metrics: impl std::fmt::Debug,
    pieces: Option<impl IntoIterator<Item = P>>,
) {
    let _ = writeln!(
        out,
        "    line {index} mode={mode:?} block_offset={} metrics={metrics:?}",
        number(block_offset),
    );
    for piece in pieces.into_iter().flatten() {
        let _ = writeln!(out, "      {}", without_font_layers(&format!("{piece:?}")));
    }
}

/// `text` without the layer number of the `FontId`s in it.
///
/// The text engine numbers the layers of fonts it registers across the whole
/// process, so the number tells how many layers the cases that ran before
/// have made, not what the case laid out. The index of the face in its layer
/// stays.
fn without_font_layers(text: &str) -> String {
    const LAYER: &str = "FontId { layer: ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(LAYER) {
        out.push_str(&rest[..at]);
        out.push_str("FontId { ");
        rest = &rest[at + LAYER.len()..];
        rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        rest = rest.strip_prefix(", ").unwrap_or(rest);
    }
    out.push_str(rest);
    out
}

fn fragment(out: &mut String, label: &str, fragment: &Fragment<'_>) {
    let content = fragment
        .content_rect()
        .map_or_else(|| "-".to_owned(), |content| rect(&content));
    let _ = writeln!(
        out,
        "  {label} node={} kind={:?} rect={} content={content} index={} fragmentainer={} \
         lines={:?} first={:?} last={:?} repeat={:?} clip={:?}",
        fragment.node().0,
        fragment.kind(),
        rect(&fragment.rect()),
        fragment.fragment_index(),
        fragment.fragmentainer(),
        fragment.line_range(),
        fragment.is_first_fragment(),
        fragment.is_last_fragment(),
        fragment.repeat(),
        fragment.overflow_clip(),
    );
}

fn text_run(out: &mut String, run: &PositionedGlyphRun<'_>) {
    let _ = writeln!(
        out,
        "  run line={:?} source={:?} font_index={} font_bytes={} size={} variations={:?} \
         coords={:?} synthesis={:?} origin={} advance={} ascent={} descent={} text={:?} \
         text_offset={} transform={:?} color={:?}",
        run.line,
        run.source,
        run.font.index,
        run.font.data.as_bytes().len(),
        number(run.font_size),
        run.variations,
        run.normalized_coords,
        run.synthesis,
        numbers(&[run.origin.0, run.origin.1]),
        number(run.advance),
        number(run.ascent),
        number(run.descent),
        run.text,
        run.text_offset,
        run.transform,
        run.color,
    );
    for glyph in &run.glyphs {
        let _ = writeln!(
            out,
            "    glyph id={} advance={} offset={} range={:?}",
            glyph.id,
            number(glyph.advance),
            numbers(&[glyph.x_offset, glyph.y_offset]),
            glyph.text_range,
        );
    }
    for decoration in &run.decorations {
        let _ = writeln!(out, "    decoration {decoration:?}");
    }
    for shadow in &run.shadows {
        let _ = writeln!(out, "    shadow {shadow:?}");
    }
}

/// The URL and rectangle a page prepared for an image, or `-` when it
/// prepared none.
fn placement(placed: Option<(&url::Url, PaintRect)>) -> String {
    placed.map_or_else(
        || "-".to_owned(),
        |(url, placed)| format!("{url} {}", rect(&placed)),
    )
}

fn paint_event(out: &mut String, event: &PaintEvent<'_>, page: &Page<'_>) {
    let document = page.paint_inputs().0;
    match event {
        PaintEvent::Box(fragment) => self::fragment(out, "box", fragment),
        PaintEvent::OuterBoxShadow(fragment) => self::fragment(out, "outer_box_shadow", fragment),
        PaintEvent::Text(fragment) => self::fragment(out, "text", fragment),
        PaintEvent::Replaced(fragment) => self::fragment(out, "replaced", fragment),
        PaintEvent::GeneratedBox(piece) => {
            let _ = writeln!(
                out,
                "  generated_box owner={} kind={:?} line={:?} rect={} clip_owner={} \
                 edges={}{}{}{}",
                piece.owner.0,
                piece.kind,
                piece.line,
                rect(&piece.rect),
                piece.clip_owner.0,
                u8::from(piece.has_start_edge),
                u8::from(piece.has_end_edge),
                u8::from(piece.has_block_start_edge),
                u8::from(piece.has_block_end_edge),
            );
        }
        PaintEvent::LineBox(piece) => {
            self::fragment(out, "line_box", &piece.fragment);
            let _ = writeln!(
                out,
                "    line={:?} rect={} edges={}{}",
                piece.line,
                rect(&piece.rect),
                u8::from(piece.has_start_edge),
                u8::from(piece.has_end_edge),
            );
        }
        // The events of images name them; where a page placed them is kept
        // in the document.
        PaintEvent::MarkerImage(owner) => {
            let _ = writeln!(
                out,
                "  marker_image owner={} placement={}",
                owner.0,
                placement(document.page_marker_image(page.index(), *owner)),
            );
        }
        PaintEvent::GeneratedImage(image) => {
            let _ = writeln!(
                out,
                "  generated_image owner={} kind={:?} index={} placement={} text_position={:?}",
                image.owner.0,
                image.kind,
                image.index,
                placement(document.page_generated_image(page.index(), *image)),
                page.generated_image_text_position(*image),
            );
        }
        // Clips, opacity and transform groups, text lines and column rules
        // hold plain data only.
        other => {
            let _ = writeln!(out, "  {other:?}");
        }
    }
}

/// Writes the box of every node of the document `page` was laid out in, in
/// arena order.
fn nodes(out: &mut String, page: &Page<'_>) {
    let (document, cascade, ..) = page.paint_inputs();
    let _ = writeln!(out, "nodes: {}", document.node_count());
    let present =
        (0..document.node_count()).filter_map(|index| Some((index, document.get_node(index)?)));
    for (index, node) in present {
        let parent = document
            .parent_of(index)
            .map_or_else(|| "-".to_owned(), |parent| parent.to_string());
        let tag = node.tag_name().unwrap_or("-");
        let mut quoted = String::new();
        if let Some(text) = node.text_content() {
            let mut chars = text.chars();
            quoted = chars.by_ref().take(QUOTED_TEXT_CHARS).collect();
            if chars.next().is_some() {
                quoted.push('…');
            }
        }
        let layout = NodeBox::of(node);
        let _ = writeln!(
            out,
            "n{index} {:?} tag={tag} text={quoted:?} parent={parent} order={} location={} \
             size={} overflow={} scrollbar={} border={} padding={} margin={}",
            node.kind(),
            layout.order,
            numbers(&layout.location),
            numbers(&layout.size),
            numbers(&layout.scrollable_overflow),
            numbers(&layout.scrollbar),
            numbers(&layout.border),
            numbers(&layout.padding),
            numbers(&layout.margin),
        );
        crate::style_view::write(out, document, cascade, index);
        // The lines of a paragraph. Text in a vertical writing mode has no
        // runs on the page, so the pieces of its lines are written here.
        for (line_index, line) in node.ifc_lines().unwrap_or_default().iter().enumerate() {
            let mode = line.writing_mode();
            shaped_line(
                out,
                line_index,
                mode,
                line.block_offset(),
                line.metrics(),
                mode.is_vertical().then(|| line.fragments()),
            );
        }
    }
}

#[cfg(test)]
mod tests;
