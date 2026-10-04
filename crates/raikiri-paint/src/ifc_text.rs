//! Paint the lines of a block laid out by the shodo inline engine.
//!
//! The lines live on the block's node (`Node::ifc_lines`). Glyph positions
//! come from `GlyphRunView::glyph_origin`, which is relative to the block's
//! content box; the caller supplies that origin in page coordinates. The
//! painter supports horizontal and vertical normal-flow lines; other geometry
//! such as vertical fragmentation and decorations remains horizontal-only.

use crate::text::{
    DecorationContext, DecorationGeometry, DecorationPhase, css_color_to_peniko,
    decorations_for_element, draw_decoration_phase, synthetic_embolden,
};
use anyrender::filters::{Filter, FilterEffect};
use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::{Affine, Rect};
use peniko::{Fill, Mix};
use raikiri_dom::Document;
use raikiri_dom::generated_content::{computed_for_id, generated_origin};
use raikiri_style::CascadeResult;
use raikiri_style::property::TextShadowColor;
use shodo::Fragment;
use shodo::geometry::{BaselineKind, PhysicalConverter, PhysicalSize, WritingMode};
use shodo::hit::{LineLayout, TextPosition};
use shodo::node::NodeId;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::Arc;

/// Content-box origin of an ifc root in page coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IfcPosition {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) shift_y: f32,
}

/// The decoration context of a text node: the context after the ifc root,
/// folded through the elements between the root and the text. Each element's
/// decoration sits at that element's baseline, `shifts` below the line's.
fn context_for_text(
    document: &Document,
    cascade: &CascadeResult,
    root_id: usize,
    text_node: usize,
    base: &DecorationContext,
    shifts: &HashMap<usize, f32>,
) -> DecorationContext {
    let mut chain = Vec::new();
    // The text of a pseudo-element is owned by the pseudo-element's own box,
    // which is decorated like an inline element child of its element.
    let mut current = match generated_origin(text_node) {
        Some((element, _)) => {
            chain.push(text_node);
            (element != root_id).then_some(element)
        }
        None => document.parent_of(text_node),
    };
    while let Some(id) = current {
        if id == root_id {
            break;
        }
        chain.push(id);
        current = document.parent_of(id);
    }
    chain.iter().rev().fold(base.clone(), |context, &id| {
        let Some(cv) = computed_for_id(cascade, id) else {
            return context;
        };
        decorations_for_element(&context, cv, shifts.get(&id).copied().unwrap_or(0.0))
    })
}

/// How far each inline element's baseline lies below the line's baseline, on
/// this line. An element with no fragment on the line is not in the map.
///
/// A fragment's content area starts at the top of its font's ascent, so the
/// element's baseline is that top plus the ascent.
fn baseline_shifts(document: &Document, line: &shodo::Line) -> HashMap<usize, f32> {
    let line_baseline = line.baseline(BaselineKind::Alphabetic);
    let mut shifts = HashMap::new();
    for fragment in line.fragments() {
        let Fragment::InlineBox(inline_box) = fragment else {
            continue;
        };
        let Some(metrics) = document.ifc_font_metrics(inline_box.font, inline_box.font_size) else {
            continue;
        };
        let baseline = inline_box.content_rect.block_start + metrics.ascent;
        shifts.insert(inline_box.node.0 as usize, baseline - line_baseline);
    }
    shifts
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
    /// The line's baseline in page coordinates.
    baseline: f64,
    decorations: DecorationContext,
    /// The relative offsets of the run's inline ancestors.
    offset: (f32, f32),
}

/// The sum of the relative offsets of `node` and its ancestors up to the ifc
/// root (the root excluded). An offset moves an inline element together with
/// everything inside it.
fn cumulative_offset(
    document: &Document,
    root_id: usize,
    offsets: &[(usize, (f32, f32))],
    node: usize,
) -> (f32, f32) {
    let (mut dx, mut dy) = (0.0, 0.0);
    // A pseudo-element moves with its element.
    let node = generated_origin(node).map_or(node, |(element, _)| element);
    let mut current = Some(node);
    while let Some(id) = current {
        if id == root_id {
            break;
        }
        if let Some((_, (x, y))) = offsets.iter().find(|(owner, _)| *owner == id) {
            dx += x;
            dy += y;
        }
        current = document.parent_of(id);
    }
    (dx, dy)
}

/// Draw the glyph runs of an ifc root.
///
/// Per line, underlines and overlines of every run come first, then the
/// glyphs, then the line-throughs (CSS Text Decoration 3 §3: underlines and
/// overlines below the text, line-throughs over it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_ifc_lines(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    root_id: usize,
    position: IfcPosition,
    base_decorations: &DecorationContext,
    fragmentainer: Option<usize>,
    custom_highlights: &[crate::TextHighlightRange],
) {
    let Some(root_node) = document.get_node(root_id) else {
        return;
    };
    let Some(lines) = root_node.ifc_lines() else {
        return;
    };
    let writing_mode = root_node
        .ifc_writing_mode()
        .unwrap_or(WritingMode::HorizontalTb);
    let physical_content_size = root_node
        .ifc_physical_content_size()
        .unwrap_or(PhysicalSize::default());
    // Lines split across columns are drawn where their column puts them:
    // the first line of a range at the range's offset. Without a split every
    // line stays where it was laid out.
    let mut line_offsets: Vec<Option<(f32, f32)>> = vec![None; lines.len()];
    match document
        .get_node(root_id)
        .and_then(|n| n.ifc_multicol_fragments())
    {
        Some(fragments) => {
            for fragment in fragments {
                if fragmentainer.is_some_and(|requested| requested != fragment.fragmentainer) {
                    continue;
                }
                let Some(first) = lines.get(fragment.line_start) else {
                    continue;
                };
                let dy = fragment.y - first.block_offset();
                let end = fragment.line_end.min(lines.len());
                for offset in &mut line_offsets[fragment.line_start.min(end)..end] {
                    *offset = Some((
                        if fragmentainer.is_some() {
                            0.0
                        } else {
                            fragment.x
                        },
                        dy,
                    ));
                }
            }
        }
        None => line_offsets.fill(Some((0.0, 0.0))),
    }
    // Lines that pagination moved down with a block before them.
    let shifts = document
        .get_node(root_id)
        .map(|n| n.ifc_line_shifts())
        .unwrap_or_default();
    for (offset, shift) in line_offsets.iter_mut().zip(&shifts) {
        if let Some((_, dy)) = offset {
            *dy += shift;
        }
    }
    let base = position;
    let size = root_node.ifc_size().unwrap_or((0.0, 0.0));
    let mut converter_size = physical_content_size;
    if writing_mode == WritingMode::HorizontalTb {
        converter_size.width = size.0;
    }
    let vertical = writing_mode != WritingMode::HorizontalTb;
    let pieces = document
        .get_node(root_id)
        .and_then(|node| node.ifc_inline_boxes())
        .unwrap_or_default();
    let mut pieces_by_line = vec![Vec::new(); lines.len()];
    for piece in pieces {
        if let Some(line_pieces) = pieces_by_line.get_mut(piece.line) {
            line_pieces.push(piece);
        }
    }
    let offsets = document
        .get_node(root_id)
        .map(|node| node.ifc_relative_offsets())
        .unwrap_or_default();
    let hit_layout = (!custom_highlights.is_empty()).then(|| LineLayout::new(lines));
    for (line_index, line) in lines.iter().enumerate() {
        let Some((column_x, column_y)) = line_offsets[line_index] else {
            continue;
        };
        let position = IfcPosition {
            x: base.x + column_x,
            y: base.y + column_y,
            shift_y: base.shift_y,
        };
        let transform = Affine::translate((
            f64::from(position.x),
            f64::from(position.y + position.shift_y),
        ));
        // The boxes of the inline elements on this line go below its text
        // (CSS 2.1 Appendix E: an inline box's background and borders, then
        // its text).
        for piece in &pieces_by_line[line_index] {
            let Some(cv) = computed_for_id(cascade, piece.node) else {
                continue;
            };
            let (dx, dy) = cumulative_offset(document, root_id, offsets, piece.node);
            // The pieces already carry the line's pagination shift, which
            // `position` holds too.
            let line_shift = shifts.get(line_index).copied().unwrap_or(0.0);
            crate::walk::paint_inline_box(
                scene,
                cv,
                piece,
                position.x + dx,
                position.y + position.shift_y + dy - line_shift,
            );
        }
        if !vertical && let Some(hit_layout) = hit_layout.as_ref() {
            draw_custom_highlights(
                scene,
                document,
                cascade,
                root_id,
                line_index,
                line,
                hit_layout,
                position,
                offsets,
                custom_highlights,
            );
        }
        // An element's shift can differ from line to line, so the contexts
        // are built per line.
        let shifts = baseline_shifts(document, line);
        let mut contexts: HashMap<usize, DecorationContext> = HashMap::new();
        let mut runs: Vec<RunDraw<'_>> = Vec::new();
        let converter = PhysicalConverter::new(writing_mode, line.used_direction(), converter_size);
        for fragment in line.fragments() {
            let Fragment::GlyphRun(run) = fragment else {
                continue;
            };
            // The paint owner is a text node; its computed color is the
            // inherited one.
            let Some(owner) = run.node() else { continue };
            let owner = owner.0 as usize;
            let Some(cv) = computed_for_id(cascade, owner) else {
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
                    let (inline, block) = run.glyph_origin(index)?;
                    let (x, y) = converter.point(inline, block + line.block_offset());
                    Some(AnyrenderGlyph { id: glyph.id, x, y })
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
            let offset = cumulative_offset(document, root_id, offsets, owner);
            let decorations = contexts
                .entry(owner)
                .or_insert_with(|| {
                    context_for_text(document, cascade, root_id, owner, base_decorations, &shifts)
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
                    + f64::from(line.baseline(BaselineKind::Alphabetic)),
                decorations,
                offset,
            });
        }
        if !vertical {
            clip_runs_to_the_line_content(line, &mut runs);
        }
        // The line content is clipped in the line's own coordinates; the
        // relative offsets move the runs afterwards.
        for run in &mut runs {
            run.x0 += f64::from(run.offset.0);
            run.x1 += f64::from(run.offset.0);
            run.baseline += f64::from(run.offset.1);
        }
        if !vertical {
            draw_decorations(scene, &runs, DecorationPhase::BeforeGlyphs);
        }
        for draw in &runs {
            let Some(font) = draw.run.font_data() else {
                continue;
            };
            let Some(owner_style) = computed_for_id(cascade, draw.owner) else {
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
            let glyph_transform =
                physical_glyph_transform(&converter, draw.run.glyph_transform(), draw.run.skew());
            let run_transform =
                transform * Affine::translate((f64::from(draw.offset.0), f64::from(draw.offset.1)));
            if !vertical {
                draw_shadows(
                    scene,
                    &ShadowRun {
                        draw,
                        font: &font,
                        coords: &coords,
                        glyph_transform,
                    },
                    owner_style,
                    run_transform,
                    IfcPosition {
                        x: position.x + draw.offset.0,
                        y: position.y + draw.offset.1,
                        shift_y: position.shift_y,
                    },
                    size,
                );
            }
            scene.draw_glyphs(
                &font,
                font_size,
                true,
                &coords,
                synthetic_embolden(draw.run.embolden(), font_size),
                Fill::NonZero,
                draw.color,
                1.0,
                run_transform,
                glyph_transform,
                draw.glyphs.clone().into_iter(),
            );
        }
        if !vertical {
            draw_decorations(scene, &runs, DecorationPhase::AfterGlyphs);
        }
    }
}

fn physical_glyph_transform(
    converter: &PhysicalConverter,
    axes: shodo::GlyphTransform,
    skew: Option<f32>,
) -> Option<Affine> {
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
    let skew = skew.map_or(Affine::IDENTITY, |degrees| {
        Affine::skew(f64::from(degrees).to_radians().tan(), 0.0)
    });
    let transform = orientation * skew;
    (transform != Affine::IDENTITY).then_some(transform)
}

#[allow(clippy::too_many_arguments)]
fn draw_custom_highlights(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    root_id: usize,
    line_index: usize,
    line: &shodo::Line,
    hit_layout: &LineLayout<'_>,
    position: IfcPosition,
    offsets: &[(usize, (f32, f32))],
    custom_highlights: &[crate::TextHighlightRange],
) {
    let line_range = line.text_range();
    let Some(offset_mapping) = line.offset_mapping() else {
        return;
    };
    for highlight in custom_highlights {
        let Some(color) = cascade.custom_highlight_styles.get(&highlight.name) else {
            continue;
        };
        let (Ok(start_byte), Ok(end_byte)) = (
            u32::try_from(highlight.start_byte),
            u32::try_from(highlight.end_byte),
        ) else {
            continue;
        };
        let node = NodeId(highlight.node as u64);
        let Some((start_offset, start_affinity)) = offset_mapping.dom_to_text(node, start_byte)
        else {
            continue;
        };
        let Some((end_offset, end_affinity)) = offset_mapping.dom_to_text(node, end_byte) else {
            continue;
        };
        if start_offset >= end_offset
            || (start_offset as usize) < line_range.start
            || (end_offset as usize) > line_range.end
        {
            continue;
        }
        let selection = hit_layout.selection_rects(
            TextPosition {
                line: line_index,
                offset: start_offset,
                affinity: start_affinity,
            },
            TextPosition {
                line: line_index,
                offset: end_offset,
                affinity: end_affinity,
            },
        );
        if selection.is_empty() {
            continue;
        }
        let color = css_color_to_peniko(*color);
        let (offset_x, offset_y) = cumulative_offset(document, root_id, offsets, highlight.node);
        for selected in selection {
            let rect = Rect::new(
                f64::from(position.x + offset_x + selected.inline_start),
                f64::from(position.y + position.shift_y + offset_y + selected.block_start),
                f64::from(position.x + offset_x + selected.inline_start + selected.inline_size),
                f64::from(
                    position.y
                        + position.shift_y
                        + offset_y
                        + selected.block_start
                        + selected.block_size,
                ),
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
        }
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
/// which isolates the blur from the rest of the page.
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

/// Stop the decoration extents of a line's runs at the end of its content.
///
/// A collapsible space at the end of a wrapped line hangs past the content;
/// a decoration does not cover it. `inline_size` is the width of the content
/// without it, but also without a punctuation mark that hangs before the line
/// start (`hang_start`), which the decoration does cover, so that is added
/// back. The content starts at the leftmost run in a left-to-right line and
/// ends at the rightmost one in a right-to-left line. A run left with no
/// extent draws no decoration.
fn clip_runs_to_the_line_content(line: &shodo::Line, runs: &mut [RunDraw<'_>]) {
    let content = f64::from(line.inline_size());
    let hang_start = f64::from(line.hang_start());
    if line.used_direction() == shodo::geometry::Direction::Rtl {
        let right = runs.iter().map(|r| r.x1).fold(f64::NEG_INFINITY, f64::max);
        for run in runs.iter_mut() {
            run.x0 = run.x0.max(right - hang_start - content);
        }
    } else {
        let left = runs.iter().map(|r| r.x0).fold(f64::INFINITY, f64::min);
        for run in runs.iter_mut() {
            run.x1 = run.x1.min(left + hang_start + content);
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
