//! Paint the lines of a block laid out by the shodo inline engine.
//!
//! The lines live on the block's node (`Node::ifc_lines`). Glyph positions
//! come from `GlyphRunView::glyph_origin`, which is relative to the block's
//! content box; the caller supplies that origin in page coordinates. Only
//! left-to-right horizontal lines are drawn: paragraphs that need anything
//! else are not assigned to the inline engine in the first place.

use crate::text::{css_color_to_peniko, synthetic_embolden};
use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::Affine;
use peniko::Fill;
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use shodo::Fragment;

/// Content-box origin of an ifc root in page coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IfcPosition {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) shift_y: f32,
}

/// Draw the glyph runs of an ifc root.
pub(crate) fn draw_ifc_lines(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    root_id: usize,
    position: IfcPosition,
) {
    let Some(lines) = document.get_node(root_id).and_then(|n| n.ifc_lines()) else {
        return;
    };
    let transform = Affine::translate((
        f64::from(position.x),
        f64::from(position.y + position.shift_y),
    ));
    for line in lines {
        for fragment in line.fragments() {
            let Fragment::GlyphRun(run) = fragment else {
                continue;
            };
            // The paint owner is a text node; its computed color is the
            // inherited one.
            let Some(owner) = run.node() else { continue };
            let Some(cv) = cascade.computed.get(owner.0 as usize) else {
                continue;
            };
            let Some(font) = run.font_data() else {
                continue;
            };
            let glyphs: Vec<AnyrenderGlyph> = run
                .glyphs()
                .enumerate()
                .filter_map(|(index, glyph)| {
                    let (x, y) = run.glyph_origin(index)?;
                    Some(AnyrenderGlyph {
                        id: glyph.id,
                        x,
                        y: y + line.block_offset(),
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
                synthetic_embolden(run.embolden(), font_size),
                Fill::NonZero,
                css_color_to_peniko(cv.color),
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
