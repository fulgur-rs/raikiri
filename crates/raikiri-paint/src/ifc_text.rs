//! Paint the lines of a block laid out by the shodo inline engine.
//!
//! The lines live on the block's node (`Node::ifc_lines`). Glyph positions
//! come from `GlyphRunView::glyph_origin`, which is relative to the block's
//! content box; the caller supplies that origin in page coordinates. Only
//! horizontal lines are drawn: paragraphs that need anything else are not
//! assigned to the inline engine in the first place. A right-to-left line
//! measures its glyph positions from the right edge of the content box.

use crate::text::{
    DecorationContext, DecorationGeometry, DecorationPhase, css_color_to_peniko,
    decorations_for_element, draw_decoration_phase, synthetic_embolden,
};
use anyrender::filters::{Filter, FilterEffect};
use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::{Affine, Rect};
use peniko::{Fill, Mix};
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use raikiri_style::property::TextShadowColor;
use shodo::Fragment;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// The text node the run belongs to.
    owner: usize,
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
    let size = document
        .get_node(root_id)
        .and_then(|n| n.ifc_size())
        .unwrap_or((0.0, 0.0));
    let content_width = size.0;
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
            let rtl = line.used_direction() == shodo::geometry::Direction::Rtl;
            let glyphs: Vec<AnyrenderGlyph> = run
                .glyphs()
                .enumerate()
                .filter_map(|(index, glyph)| {
                    let (inline, y) = run.glyph_origin(index)?;
                    // In a right-to-left line the origin is the distance from
                    // the inline-start (right) edge to the glyph's far edge.
                    let x = if rtl { content_width - inline } else { inline };
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
            // The run spans from its leftmost glyph origin to the right end
            // of its rightmost advance, in either direction.
            let mut first_x = f64::INFINITY;
            let mut last_x = f64::NEG_INFINITY;
            for (glyph, shaped) in glyphs.iter().zip(run.glyphs()) {
                first_x = first_x.min(f64::from(glyph.x));
                last_x = last_x.max(f64::from(glyph.x) + f64::from(shaped.advance));
            }
            let decorations = contexts
                .entry(owner)
                .or_insert_with(|| {
                    context_for_text(document, cascade, root_id, owner, base_decorations)
                })
                .clone();
            runs.push(RunDraw {
                run,
                owner,
                color: css_color_to_peniko(cv.color),
                glyphs,
                x0: f64::from(position.x) + first_x,
                x1: f64::from(position.x) + last_x,
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
            draw_shadows(
                scene,
                &ShadowRun {
                    draw,
                    font: &font,
                    coords: &coords,
                    glyph_transform,
                },
                &cascade.computed[draw.owner],
                transform,
                position,
                size,
            );
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

/// A glyph run with the font data the scene needs to draw it again.
struct ShadowRun<'a, 'b> {
    draw: &'a RunDraw<'b>,
    font: &'a peniko::FontData,
    coords: &'a [i16],
    glyph_transform: Option<Affine>,
}

/// Draw the text shadows of one run, behind its glyphs.
///
/// CSS Text Decoration 3 §4 paints the shadows below the text, the first one
/// on top, so the list is drawn in reverse. A blurred shadow is drawn inside a
/// filter layer clipped to the lines grown by three times the blur radius,
/// which is how the parley path isolates the blur.
fn draw_shadows(
    scene: &mut impl PaintScene,
    run: &ShadowRun<'_, '_>,
    owner_cv: &raikiri_style::ComputedValues,
    transform: Affine,
    position: IfcPosition,
    (width, height): (f32, f32),
) {
    let font_size = run.draw.run.font_size();
    for shadow in owner_cv.text_shadow.iter().rev() {
        let color = match shadow.color {
            TextShadowColor::CurrentColor => owner_cv.color,
            TextShadowColor::Resolved(color) => color,
            _ => owner_cv.color,
        };
        let (dx, dy) = (
            f64::from(shadow.offset_x.px()),
            f64::from(shadow.offset_y.px()),
        );
        let shadow_transform = transform * Affine::translate((dx, dy));
        let blur = shadow.blur_radius.px();
        let use_blur_layer = blur.is_finite() && blur > 0.0;
        if use_blur_layer {
            let extent = f64::from((blur * 3.0).max(1.0));
            let (x, y) = (f64::from(position.x), f64::from(position.y));
            let clip = Rect::new(
                x + dx - extent,
                y + dy - extent,
                x + f64::from(width) + dx + extent,
                y + f64::from(height) + dy + extent,
            );
            scene.push_layer(
                Mix::Normal,
                1.0,
                Affine::IDENTITY,
                &clip,
                Some(Arc::new(Filter::single(FilterEffect::blur(blur)))),
                None,
            );
        }
        scene.draw_glyphs(
            run.font,
            font_size,
            true,
            run.coords,
            synthetic_embolden(run.draw.run.embolden(), font_size),
            Fill::NonZero,
            css_color_to_peniko(color),
            1.0,
            shadow_transform,
            run.glyph_transform,
            run.draw.glyphs.clone().into_iter(),
        );
        if use_blur_layer {
            scene.pop_layer();
        }
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
