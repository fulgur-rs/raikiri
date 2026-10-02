//! Text that is not part of a paragraph (page margin boxes, generated
//! pseudo-element text, list markers), shaped by the inline engine with the
//! document's fonts.

use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::Affine;
use peniko::{Color, Fill};
use raikiri_dom::{Document, StandaloneAlign, StandaloneStyle, StandaloneText};
use shodo::Fragment;
use shodo::geometry::PhysicalConverter;

/// The family names of a CSS `font-family` string, in order. Quotes are kept:
/// the engine side tells a quoted `"serif"` (a named family) from the generic
/// keyword.
fn families(list: &str) -> Vec<String> {
    list.split(',')
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// The font size a run is shaped at: `font_size`, or 16px when it is not
/// finite and positive.
pub(crate) fn usable_size(font_size: f32) -> f32 {
    if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    }
}

/// The shaped run, or `None` when there is nothing to draw: `content` is
/// empty or exceeds the engine's limits.
pub(crate) fn shape(
    document: &Document,
    content: &str,
    font_size: f32,
    font_family: &str,
    width: Option<f32>,
    align: StandaloneAlign,
) -> Option<StandaloneText> {
    let style = style(font_size, font_family);
    document.shape_standalone_text(content, &style, width, align)
}

/// Horizontal defaults for callers that only know the font.
pub(crate) fn style(font_size: f32, font_family: &str) -> StandaloneStyle {
    StandaloneStyle {
        families: families(font_family),
        font_size: usable_size(font_size),
        ..StandaloneStyle::default()
    }
}

/// Draw the glyph runs of a shaped run with its origin at (`x`, `y`).
///
/// Convert logical positions and outline vectors separately so RTL never
/// mirrors outlines and vertical characters retain their orientation.
pub(crate) fn draw(
    scene: &mut impl PaintScene,
    shaped: &StandaloneText,
    x: f32,
    y: f32,
    color: Color,
) {
    let transform = Affine::translate((f64::from(x), f64::from(y)));
    for (index, line) in shaped.lines().iter().enumerate() {
        let converter = PhysicalConverter::new(
            line.writing_mode(),
            line.used_direction(),
            shaped.container(),
        );
        let shift = shaped.hang_shift(index);
        for fragment in line.fragments() {
            let Fragment::GlyphRun(run) = fragment else {
                continue;
            };
            let Some(font) = run.font_data() else {
                continue;
            };
            let glyphs: Vec<AnyrenderGlyph> = run
                .glyphs()
                .enumerate()
                .filter_map(|(index, glyph)| {
                    let (gx, gy) = run.glyph_origin(index)?;
                    let (x, y) = converter.point(gx + shift, gy + line.block_offset());
                    Some(AnyrenderGlyph { id: glyph.id, x, y })
                })
                .collect();
            if glyphs.is_empty() {
                continue;
            }
            let font_size = run.font_size();
            // shodo's normalized coordinates are `F2Dot14` newtypes; the scene
            // takes the raw `i16` bits.
            let coords: Vec<i16> = run
                .normalized_coords()
                .iter()
                .map(|coord| coord.to_bits())
                .collect();
            let axes = run.glyph_transform();
            let (xx, yx) = converter.vector(axes.inline_x, axes.block_x);
            let (xy, yy) = converter.vector(axes.inline_y, axes.block_y);
            let orientation = Affine::new([
                f64::from(xx),
                f64::from(yx),
                f64::from(xy),
                f64::from(yy),
                0.0,
                0.0,
            ]);
            let skew = run.skew().map_or(Affine::IDENTITY, |degrees| {
                Affine::skew(f64::from(degrees).to_radians().tan(), 0.0)
            });
            let glyph_transform =
                (orientation * skew != Affine::IDENTITY).then_some(orientation * skew);
            scene.draw_glyphs(
                &font,
                font_size,
                true,
                &coords,
                crate::text::synthetic_embolden(run.embolden(), font_size),
                Fill::NonZero,
                color,
                1.0,
                transform,
                glyph_transform,
                glyphs.into_iter(),
            );
        }
    }
}

#[cfg(test)]
mod tests;
