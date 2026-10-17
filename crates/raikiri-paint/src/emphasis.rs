//! Emphasis marks (`text-emphasis`, CSS Text Decoration 3 §3).
//!
//! The inline engine reserves the space for the marks and reports, per glyph
//! run, the mark character, its size, its side and the distance from the
//! baseline to the edge the mark is painted outside. This module places one
//! mark over each typographic character that takes one, centered on the
//! character in the inline direction; a shared shaping cluster such as a
//! ligature divides its advance evenly among its characters, as Blink does.
//! A horizontal mark is placed as a one-character ruby annotation would be:
//! its line box rests on that edge and it is centered in the same fixed-point
//! units, so marks and the equivalent ruby land on the same pixels.
//! Marks stay upright in vertical text (CSS Text Decoration 3 §3.1), so a
//! vertical run's marks are shaped as upright vertical text and centered on
//! their character's em box.

use anyrender::PaintScene;
use peniko::Color;
use raikiri_dom::{Document, StandaloneAlign, StandaloneText};
use raikiri_style::ComputedValues;
use raikiri_style::property::TextDecorationColor;
use shodo::Fragment;
use shodo::geometry::{Direction, PhysicalConverter, WritingMode};

/// The mark shaped once per run, with what placing it needs.
struct ShapedMark {
    text: StandaloneText,
    /// Inline advance of the mark.
    advance: f32,
    /// Block-start edge of the mark's line box from the shaped text's origin.
    line_start: f32,
    /// Block-end edge of the mark's line box from the shaped text's origin.
    line_end: f32,
}

fn shape_mark(
    document: &Document,
    character: char,
    font_size: f32,
    families: &[raikiri_style::FontFamilyName],
) -> Option<ShapedMark> {
    let mut content = [0u8; 4];
    let text = crate::standalone_text::shape(
        document,
        character.encode_utf8(&mut content),
        font_size,
        families,
        None,
        StandaloneAlign::Start,
    )?; // cov:ignore: one character is never empty nor over the engine's limits.
    let line = text.lines().first()?;
    let run = line.fragments().find_map(|fragment| match fragment {
        Fragment::GlyphRun(run) => Some(run),
        _ => None,
    })?;
    let advance = run.inline_size();
    Some(ShapedMark {
        advance,
        line_start: line.block_offset(),
        line_end: line.block_offset() + line.block_size(),
        text,
    })
}

/// Draw the emphasis marks of one horizontal glyph run.
///
/// `glyphs` are the run's glyph positions in line coordinates, drawn from
/// the page x `origin_x`; `baseline` is the run's baseline in page
/// coordinates. `text_color` is used for `currentcolor`.
pub(crate) fn draw_marks(
    scene: &mut impl PaintScene,
    document: &Document,
    run: &shodo::GlyphRunView<'_>,
    glyphs: &[raikiri_dom::LineGlyph],
    (origin_x, baseline): (f32, f32),
    style: &ComputedValues,
    text_color: Color,
) {
    let color = match style.text_emphasis_color {
        TextDecorationColor::Resolved(color) => crate::text::css_color_to_peniko(color),
        _ => text_color,
    };
    draw_marks_in(
        scene,
        document,
        run,
        glyphs,
        (origin_x, baseline),
        style,
        color,
    );
}

/// [`draw_marks`] with every mark filled in `color`, as a text shadow
/// draws them.
pub(crate) fn draw_marks_in(
    scene: &mut impl PaintScene,
    document: &Document,
    run: &shodo::GlyphRunView<'_>,
    glyphs: &[raikiri_dom::LineGlyph],
    (origin_x, baseline): (f32, f32),
    style: &ComputedValues,
    color: Color,
) {
    let Some(mark) = run.emphasis_mark() else {
        return;
    };
    let Some(shaped) = shape_mark(document, mark.character, mark.font_size, &style.font_family)
    else {
        return; // cov:ignore: shaping one character fails only past the engine's limits.
    };
    for cluster in run.clusters() {
        // The cluster spans from its leftmost glyph origin to the end of its
        // rightmost advance, in either direction.
        let mut start = f32::INFINITY;
        let mut end = f32::NEG_INFINITY;
        let mut found = false;
        for glyph in glyphs {
            // A glyph records the text offset of its character, which falls
            // inside the text range of the cluster it belongs to.
            if !cluster.text_range.contains(&(glyph.cluster as usize)) {
                continue;
            }
            start = start.min(glyph.x);
            end = end.max(glyph.x + glyph.advance);
            found = true;
        }
        if !found {
            continue; // cov:ignore: every cluster of a run has a glyph in that run.
        }
        // The mark's line box rests on the edge `offset` from the run's
        // baseline as a ruby annotation's line rests on its base. The run's
        // baseline is used rather than a glyph's origin, which carries the
        // glyph's own vertical offset.
        let mark_top = if mark.line_over {
            baseline - mark.offset - shaped.line_end
        } else {
            baseline + mark.offset - shaped.line_start
        };
        // A right-to-left run lays its characters out from the right.
        let reversed = run.bidi_level() % 2 == 1;
        for slot in run.emphasis_slots(&cluster) {
            let (share_start, share) = slot_share(slot, reversed, end - start);
            crate::standalone_text::draw(
                scene,
                &shaped.text,
                origin_x + start + share_start + centered_offset(share, shaped.advance),
                mark_top,
                color,
            );
        }
    }
}

/// Where `slot`'s character starts along a cluster spanning `span`, as an
/// offset from the cluster's start in the direction its glyphs are laid out,
/// and the share of the span it takes. A shared cluster divides its advance
/// evenly among its characters.
fn slot_share(slot: shodo::EmphasisSlot, reversed: bool, span: f32) -> (f32, f32) {
    let share = span / slot.count as f32;
    (share * slot_index(slot, reversed) as f32, share)
}

/// Offset that centers a mark of `advance` in `width`, rounded as the
/// inline engine centers a ruby annotation over its base: the free space is
/// halved in 1/64 px units, truncating toward zero, so a mark lands on the
/// same pixels as the equivalent annotation.
fn centered_offset(width: f32, advance: f32) -> f32 {
    let unit = |value: f32| (f64::from(value) * 64.0).round() as i64;
    let extra = unit(width) - unit(advance);
    (extra / 2) as f32 / 64.0
}

fn slot_index(slot: shodo::EmphasisSlot, reversed: bool) -> u32 {
    if reversed {
        slot.count - 1 - slot.index
    } else {
        slot.index
    }
}

/// Where `slot`'s character is centered along its cluster's advance, as a
/// fraction from the cluster's start in the direction its glyphs are laid
/// out. A shared cluster divides its advance evenly among its characters.
fn slot_center(slot: shodo::EmphasisSlot, reversed: bool) -> f32 {
    (slot_index(slot, reversed) as f32 + 0.5) / slot.count as f32
}

/// The mark shaped as upright vertical text, with the physical offset of its
/// em box center from the shaped text's origin.
struct UprightMark {
    text: StandaloneText,
    center: (f32, f32),
}

fn shape_upright_mark(
    document: &Document,
    character: char,
    font_size: f32,
    families: &[raikiri_style::FontFamilyName],
) -> Option<UprightMark> {
    let mut content = [0u8; 4];
    let style = raikiri_dom::StandaloneStyle {
        writing_mode: WritingMode::VerticalRl,
        text_orientation: shodo::style::TextOrientation::Upright,
        ..crate::standalone_text::style(font_size, families)
    };
    let text = document.shape_standalone_text(
        character.encode_utf8(&mut content),
        &style,
        None,
        StandaloneAlign::Start,
    )?; // cov:ignore: one character is never empty nor over the engine's limits.
    let line = text.lines().first()?;
    let run = line.fragments().find_map(|fragment| match fragment {
        Fragment::GlyphRun(run) => Some(run),
        _ => None,
    })?;
    let converter =
        PhysicalConverter::new(line.writing_mode(), line.used_direction(), text.container());
    // An upright run's baseline is the central one, through its em box center.
    let center = converter.point(
        run.inline_start() + run.inline_size() / 2.0,
        line.block_offset() + run.baseline(),
    );
    Some(UprightMark { text, center })
}

/// Draw the emphasis marks of one glyph run of a vertical line.
///
/// `converter` maps the logical coordinates of `line`, laid out in
/// `writing_mode`, to the ifc root's, whose page origin is `origin`. Each
/// mark's em box is centered on its character's share of its cluster's
/// advance and starts the mark's offset from the run's baseline toward its
/// side. A run of
/// `text-combine-upright` text takes one mark per combined square it starts,
/// centered on the square, as its characters take none of their own.
pub(crate) fn draw_vertical_marks(
    scene: &mut impl PaintScene,
    document: &Document,
    (line, run): (&shodo::Line, &shodo::GlyphRunView<'_>),
    (writing_mode, converter): (WritingMode, &PhysicalConverter),
    origin: (f32, f32),
    style: &ComputedValues,
    text_color: Color,
) {
    let Some(mark) = run.emphasis_mark() else {
        return;
    };
    let color = match style.text_emphasis_color {
        TextDecorationColor::Resolved(color) => crate::text::css_color_to_peniko(color),
        _ => text_color,
    };
    let Some(shaped) =
        shape_upright_mark(document, mark.character, mark.font_size, &style.font_family)
    else {
        return; // cov:ignore: shaping one character fails only past the engine's limits.
    };
    // Line-over is the block-start side, except in `vertical-lr`, whose
    // lines stack left to right with their line-over side on the right.
    let over = if writing_mode == WritingMode::VerticalLr {
        1.0
    } else {
        -1.0
    };
    let side = if mark.line_over { over } else { -over };
    let reach = side * (mark.offset + mark.font_size / 2.0);
    let mut draw = |inline: f32, block: f32| {
        let (x, y) = converter.point(inline, block);
        crate::standalone_text::draw(
            scene,
            &shaped.text,
            origin.0 + x - shaped.center.0,
            origin.1 + y - shaped.center.1,
            color,
        );
    };
    if run.orientation() == shodo::GlyphOrientation::Combined {
        // A square can span several runs, and its first characters may have
        // no glyph (a preserved tab); only the earliest run inside it marks
        // it, so it takes one mark.
        let text = run.text_range();
        for combination in line.text_combinations() {
            let range = &combination.text_range;
            let first = line
                .fragments()
                .filter_map(|fragment| match fragment {
                    Fragment::GlyphRun(run) => Some(run.text_range()),
                    _ => None,
                })
                .filter(|run| run.start < range.end && range.start < run.end)
                .min_by_key(|run| run.start);
            if first == Some(text.clone()) {
                let square = combination.square;
                draw(
                    square.inline_start + square.inline_size / 2.0,
                    line.block_offset() + square.block_start + square.block_size / 2.0 + reach,
                );
            }
        }
        return;
    }
    // Clusters follow the run's text, which bidi reordering can reverse;
    // lay their advances out in the order their glyphs are drawn.
    let glyphs: Vec<shodo::Glyph> = run.glyphs().collect();
    let mut clusters: Vec<(f32, shodo::Cluster)> = run
        .clusters()
        .filter_map(|cluster| {
            let key = glyphs
                .iter()
                .filter(|glyph| cluster.text_range.contains(&(glyph.cluster as usize)))
                .map(|glyph| glyph.inline_position)
                .reduce(f32::min)?;
            Some((key, cluster))
        })
        .collect();
    clusters.sort_by(|a, b| a.0.total_cmp(&b.0));
    let block = line.block_offset() + run.baseline() + reach;
    // A run against the line's direction lays its characters out backward.
    let reversed = (run.bidi_level() % 2 == 1) != (line.used_direction() == Direction::Rtl);
    let mut inline = run.inline_start();
    for (_, cluster) in clusters {
        for slot in run.emphasis_slots(&cluster) {
            draw(
                inline + cluster.advance * slot_center(slot, reversed),
                block,
            );
        }
        inline += cluster.advance;
    }
}

#[cfg(test)]
mod tests;
