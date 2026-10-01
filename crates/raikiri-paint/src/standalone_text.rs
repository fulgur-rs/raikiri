//! Text that is not part of a paragraph (page margin boxes, generated
//! pseudo-element text, list markers), shaped by the inline engine when the
//! document has one.

use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::Affine;
use peniko::{Color, Fill};
use raikiri_dom::{Document, StandaloneAlign, StandaloneStyle, StandaloneText};
use shodo::Fragment;

/// The family names of a CSS `font-family` string, in order. Quotes are kept:
/// the engine side tells a quoted `"serif"` (a named family) from the generic
/// keyword.
fn families(list: &str) -> Vec<String> {
    list.split(',')
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// The size the parley path uses for an unusable value.
pub(crate) fn usable_size(font_size: f32) -> f32 {
    if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        16.0
    }
}

/// The shaped run when the document's engine takes `content`, else `None`:
/// the caller then uses the parley path.
pub(crate) fn shape(
    document: Option<&Document>,
    content: &str,
    font_size: f32,
    font_family: &str,
    width: Option<f32>,
    align: StandaloneAlign,
) -> Option<StandaloneText> {
    let style = StandaloneStyle {
        families: families(font_family),
        font_size: usable_size(font_size),
    };
    document?.shape_standalone_text(content, &style, width, align)
}

/// Draw the glyph runs of a shaped run with its origin at (`x`, `y`).
///
/// Glyph origins come from the engine's lines, which are left to right here:
/// right-to-left text never reaches the engine.
pub(crate) fn draw(
    scene: &mut impl PaintScene,
    shaped: &StandaloneText,
    x: f32,
    y: f32,
    color: Color,
) {
    let transform = Affine::translate((f64::from(x), f64::from(y)));
    for (index, line) in shaped.lines().iter().enumerate() {
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
                    Some(AnyrenderGlyph {
                        id: glyph.id,
                        x: gx + shift,
                        y: gy + line.block_offset(),
                    })
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
            let glyph_transform = run
                .skew()
                .map(|degrees| Affine::skew(f64::from(degrees).to_radians().tan(), 0.0));
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
