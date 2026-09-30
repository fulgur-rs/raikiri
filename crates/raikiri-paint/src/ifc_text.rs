//! Paint the lines of a block laid out by the shodo inline engine.
//!
//! The lines live on the block's node (`Node::ifc_lines`). Glyph positions
//! come from `GlyphRunView::glyph_origin`, which is relative to the block's
//! content box; the caller supplies that origin in page coordinates. Only
//! left-to-right horizontal lines are drawn: paragraphs that need anything
//! else are not assigned to the inline engine in the first place.

use crate::text::{
    DecorationContext, DecorationGeometry, DecorationPhase, css_color_to_peniko,
    decorations_for_element, draw_decoration_phase, synthetic_embolden,
};
use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::Affine;
use peniko::Fill;
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use shodo::Fragment;
use std::collections::HashMap;

/// Content-box origin of an ifc root in page coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IfcPosition {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) shift_y: f32,
}

/// The decoration context of a text node: the context after the ifc root,
/// folded through the elements between the root and the text.
fn context_for_text(
    document: &Document,
    cascade: &CascadeResult,
    root_id: usize,
    text_node: usize,
    base: &DecorationContext,
) -> DecorationContext {
    let mut chain = Vec::new();
    let mut current = document.parent_of(text_node);
    while let Some(id) = current {
        if id == root_id {
            break;
        }
        chain.push(id);
        current = document.parent_of(id);
    }
    chain.iter().rev().fold(base.clone(), |context, &id| {
        decorations_for_element(&context, &cascade.computed[id], 0.0)
    })
}

/// One glyph run ready to draw, with its decoration context.
struct RunDraw<'a> {
    run: shodo::GlyphRunView<'a>,
    color: peniko::Color,
    glyphs: Vec<AnyrenderGlyph>,
    /// Horizontal extent of the run in page coordinates.
    x0: f64,
    x1: f64,
    /// Baseline in page coordinates.
    baseline: f64,
    decorations: DecorationContext,
}

/// Draw the glyph runs of an ifc root.
///
/// Per line, underlines and overlines of every run come first, then the
/// glyphs, then the line-throughs, which is the order the parley path uses.
pub(crate) fn draw_ifc_lines(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    root_id: usize,
    position: IfcPosition,
    base_decorations: &DecorationContext,
) {
    let Some(lines) = document.get_node(root_id).and_then(|n| n.ifc_lines()) else {
        return;
    };
    let transform = Affine::translate((
        f64::from(position.x),
        f64::from(position.y + position.shift_y),
    ));
    let mut contexts: HashMap<usize, DecorationContext> = HashMap::new();
    for line in lines {
        let mut runs: Vec<RunDraw<'_>> = Vec::new();
        for fragment in line.fragments() {
            let Fragment::GlyphRun(run) = fragment else {
                continue;
            };
            // The paint owner is a text node; its computed color is the
            // inherited one.
            let Some(owner) = run.node() else { continue };
            let owner = owner.0 as usize;
            let Some(cv) = cascade.computed.get(owner) else {
                continue;
            };
            let Some(font) = run.font_data() else {
                continue;
            };
            let _ = font;
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
            // Lines are left to right, so the run spans from its first glyph
            // origin to the end of its last advance.
            let count = glyphs.len();
            let first_x = f64::from(glyphs[0].x);
            let last_advance = run
                .glyphs()
                .get(count - 1)
                .map_or(0.0, |glyph| f64::from(glyph.advance));
            let last_x = f64::from(glyphs[count - 1].x);
            let decorations = contexts
                .entry(owner)
                .or_insert_with(|| {
                    context_for_text(document, cascade, root_id, owner, base_decorations)
                })
                .clone();
            runs.push(RunDraw {
                run,
                color: css_color_to_peniko(cv.color),
                glyphs,
                x0: f64::from(position.x) + first_x,
                x1: f64::from(position.x) + last_x + last_advance,
                baseline: f64::from(position.y + position.shift_y)
                    + f64::from(line.block_offset())
                    + f64::from(run.baseline()),
                decorations,
            });
        }
        draw_decorations(scene, &runs, DecorationPhase::BeforeGlyphs);
        for draw in &runs {
            let Some(font) = draw.run.font_data() else {
                continue;
            };
            let font_size = draw.run.font_size();
            // shodo's normalized coordinates are `F2Dot14` newtypes; the scene
            // takes the raw `i16` bits.
            let coords: Vec<i16> = draw
                .run
                .normalized_coords()
                .iter()
                .map(|coord| coord.to_bits())
                .collect();
            let glyph_transform = draw
                .run
                .skew()
                .map(|degrees| Affine::skew(f64::from(degrees).to_radians().tan(), 0.0));
            scene.draw_glyphs(
                &font,
                font_size,
                true,
                &coords,
                synthetic_embolden(draw.run.embolden(), font_size),
                Fill::NonZero,
                draw.color,
                1.0,
                transform,
                glyph_transform,
                draw.glyphs.clone().into_iter(),
            );
        }
        draw_decorations(scene, &runs, DecorationPhase::AfterGlyphs);
    }
}

fn draw_decorations(scene: &mut impl PaintScene, runs: &[RunDraw<'_>], phase: DecorationPhase) {
    for draw in runs {
        let specs = draw.decorations.specs();
        if specs.is_empty() {
            continue;
        }
        let geometry = DecorationGeometry {
            x0: draw.x0,
            x1: draw.x1,
            abs_y: draw.baseline,
            line_top: 0.0,
            baseline: Some(draw.baseline),
        };
        draw_decoration_phase(scene, &specs, geometry, phase);
    }
}

#[cfg(test)]
mod tests;
