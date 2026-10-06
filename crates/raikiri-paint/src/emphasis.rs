//! Emphasis marks (`text-emphasis`, CSS Text Decoration 3 §3).
//!
//! The inline engine reserves the space for the marks and reports, per glyph
//! run, the mark character, its size, its side and the distance from the
//! baseline to the edge the mark is painted outside. This module places one
//! mark over each shaping cluster that takes one, centered on the cluster in
//! the inline direction, as Blink does.

use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use peniko::Color;
use raikiri_dom::{Document, StandaloneAlign, StandaloneText};
use raikiri_style::ComputedValues;
use raikiri_style::property::TextDecorationColor;
use shodo::Fragment;
use shodo::geometry::BaselineKind;

/// The mark shaped once per run, with what placing it needs.
struct ShapedMark {
    text: StandaloneText,
    /// Inline advance of the mark.
    advance: f32,
    /// Baseline of the shaped mark from its own origin.
    baseline: f32,
    /// Part of the mark's em box above its baseline.
    em_ascent: f32,
}

fn shape_mark(
    document: &Document,
    character: char,
    font_size: f32,
    families: &str,
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
    let metrics = run.metrics();
    let height = metrics.ascent + metrics.descent;
    // The engine places the mark's em box, so split one em between ascent and
    // descent in the face's own proportion.
    let em_ascent = if height > 0.0 {
        font_size * metrics.ascent / height
    } else {
        font_size * 0.8 // cov:ignore: only a face with no vertical metrics has no height.
    };
    let advance = run.inline_size();
    let baseline = line.block_offset() + line.baseline(BaselineKind::Alphabetic);
    Some(ShapedMark {
        advance,
        baseline,
        em_ascent,
        text,
    })
}

/// The CSS `font-family` list of `style`, for shaping the mark in the same
/// fonts as the emphasized text.
fn families(style: &ComputedValues) -> String {
    let names: Vec<&str> = style
        .font_family
        .iter()
        .map(|family| family.as_str())
        .collect();
    names.join(", ")
}

/// Draw the emphasis marks of one horizontal glyph run.
///
/// `glyphs` are the run's glyph positions in line coordinates, drawn at
/// `origin` in page coordinates, and `baseline` is the run's baseline in the
/// same coordinates; `text_color` is used for `currentcolor`.
pub(crate) fn draw_marks(
    scene: &mut impl PaintScene,
    document: &Document,
    run: &shodo::GlyphRunView<'_>,
    glyphs: &[AnyrenderGlyph],
    origin: (f32, f32),
    baseline: f32,
    style: &ComputedValues,
    text_color: Color,
) {
    let Some(mark) = run.emphasis_mark() else {
        return;
    };
    let Some(shaped) = shape_mark(document, mark.character, mark.font_size, &families(style))
    else {
        return; // cov:ignore: shaping one character fails only past the engine's limits.
    };
    let color = match style.text_emphasis_color {
        TextDecorationColor::Resolved(color) => crate::text::css_color_to_peniko(color),
        _ => text_color,
    };
    let advances: Vec<f32> = run.glyphs().map(|glyph| glyph.advance).collect();
    // A glyph records the text offset of its character, which falls inside
    // the text range of the cluster it belongs to.
    let offsets: Vec<usize> = run.glyphs().map(|glyph| glyph.cluster as usize).collect();
    for cluster in run.clusters() {
        if cluster.flags.emphasis_excluded {
            continue;
        }
        // The cluster spans from its leftmost glyph origin to the end of its
        // rightmost advance, in either direction.
        let mut start = f32::INFINITY;
        let mut end = f32::NEG_INFINITY;
        let mut found = false;
        for ((glyph, advance), offset) in glyphs.iter().zip(&advances).zip(&offsets) {
            if !cluster.text_range.contains(offset) {
                continue;
            }
            start = start.min(glyph.x);
            end = end.max(glyph.x + advance);
            found = true;
        }
        if !found {
            continue; // cov:ignore: every cluster of a run has a glyph in that run.
        }
        let center = origin.0 + (start + end) / 2.0;
        // The mark is placed from the run's baseline, not from a glyph's
        // origin, which carries the glyph's own vertical offset.
        let text_baseline = origin.1 + baseline;
        // The mark's em box starts `offset` from the baseline toward its side.
        let mark_baseline = if mark.line_over {
            text_baseline - mark.offset - (mark.font_size - shaped.em_ascent)
        } else {
            text_baseline + mark.offset + shaped.em_ascent
        };
        crate::standalone_text::draw(
            scene,
            &shaped.text,
            center - shaped.advance / 2.0,
            mark_baseline - shaped.baseline,
            color,
        );
    }
}
