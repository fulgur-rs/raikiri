//! Paint the lines of a block laid out by the shodo inline engine.
//!
//! The lines live on the block's node (`Node::ifc_lines`). Glyph positions
//! come from `raikiri_dom::PositionedLines`, relative to the block's content
//! box, the same positions the page text runs report; the caller supplies
//! that origin in page coordinates. The
//! painter supports horizontal and vertical normal-flow lines; other geometry
//! such as vertical fragmentation and decorations remains horizontal-only.

use crate::text::{
    DecorationContext, DecorationPhase, css_color_to_peniko, draw_resolved_decoration_phase,
    synthetic_embolden,
};
use anyrender::filters::{Filter, FilterEffect};
use anyrender::{Glyph as AnyrenderGlyph, PaintScene};
use kurbo::{Affine, Rect};
use peniko::{Fill, Mix};
use raikiri_dom::generated_content::{computed_for_id, generated_origin};
use raikiri_dom::{Document, PositionedLines, cumulative_offset};
use raikiri_style::CascadeResult;
use raikiri_style::property::TextShadowColor;
use shodo::Fragment;
use shodo::geometry::{PhysicalConverter, WritingMode};
use shodo::hit::{LineLayout, TextPosition};
use shodo::node::NodeId;
use std::convert::TryFrom;
use std::sync::Arc;

mod typographic;
pub(crate) use typographic::BackgroundSlice;

/// Content-box origin of an ifc root in page coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IfcPosition {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) shift_y: f32,
}

/// One glyph run ready to draw, with its decoration context.
struct RunDraw<'a> {
    run: shodo::GlyphRunView<'a>,
    style: &'a raikiri_style::ComputedValues,
    color: peniko::Color,
    glyphs: Vec<AnyrenderGlyph>,
    decorations: Vec<raikiri_dom::DecorationLine>,
    /// The relative offsets of the run's inline ancestors.
    offset: (f32, f32),
    group: Option<usize>,
}

/// Draw the glyph runs of an ifc root.
///
/// Per line, underlines and overlines of every run come first, then the
/// glyphs, then the line-throughs (CSS Text Decoration 3 §3: underlines and
/// overlines below the text, line-throughs over it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_ifc_lines_with_resources(
    scene: &mut impl PaintScene,
    document: &Document,
    cascade: &CascadeResult,
    root_id: usize,
    position: IfcPosition,
    base_decorations: &DecorationContext,
    fragmentainer: Option<usize>,
    custom_highlights: &[crate::TextHighlightRange],
    pixel_source: Option<&dyn raikiri_traits::ImagePixelSource>,
    warnings: &mut Vec<raikiri_traits::RenderWarning>,
    page_box: raikiri_traits::PageBox,
    paint_transform: Affine,
) {
    let Some(positioned) = PositionedLines::new(document, cascade, root_id, fragmentainer) else {
        return;
    };
    let writing_mode = positioned.writing_mode;
    let shifts = &positioned.shifts;
    let base = position;
    let size = positioned.size;
    let vertical = writing_mode != WritingMode::HorizontalTb;
    let pieces = document
        .get_node(root_id)
        .and_then(|node| node.ifc_inline_boxes())
        .unwrap_or_default();
    let mut pieces_by_line = vec![Vec::new(); positioned.all_lines().len()];
    for piece in pieces {
        if let Some(line_pieces) = pieces_by_line.get_mut(piece.line) {
            line_pieces.push(piece);
        }
    }
    let offsets = positioned.offsets;
    let hit_layout =
        (!custom_highlights.is_empty()).then(|| LineLayout::new(positioned.all_lines()));
    for positioned_line in positioned.lines() {
        let line_index = positioned_line.index;
        let line = positioned_line.line;
        let (column_x, column_y) = positioned_line.offset;
        let position = IfcPosition {
            x: base.x + column_x,
            y: base.y + column_y,
            shift_y: base.shift_y,
        };
        let transform = Affine::translate((
            f64::from(position.x),
            f64::from(position.y + position.shift_y),
        ));
        let root_node = document
            .get_node(root_id)
            .expect("positioned lines require an existing IFC root");
        let mut paint = typographic::TypographicPaint::new(root_node, &pieces_by_line[line_index]);
        let background_slices = typographic::background_slices(&pieces_by_line[line_index]);
        for fragment in line.fragments() {
            let Fragment::Atomic(atomic) = fragment else {
                continue;
            };
            let Some((element, raikiri_style::PseudoElem::Marker)) =
                generated_origin(atomic.node.0 as usize)
            else {
                continue;
            };
            let Some(image) = document.list_marker_image(element) else {
                continue;
            };
            if computed_for_id(cascade, atomic.node.0 as usize)
                .is_some_and(|cv| cv.visibility != raikiri_style::property::Visibility::Visible)
            {
                continue;
            }
            let mut logical = atomic.border_rect;
            logical.block_start += line.block_offset();
            let rect = positioned_line.converter.rect(logical);
            let brush = peniko::ImageBrush::new(peniko::ImageData {
                data: peniko::Blob::from(image.rgba.clone()),
                format: peniko::ImageFormat::Rgba8,
                alpha_type: peniko::ImageAlphaType::Alpha,
                width: image.width,
                height: image.height,
            });
            paint.target(None).fill(
                Fill::NonZero,
                Affine::translate((
                    f64::from(position.x + rect.x),
                    f64::from(position.y + position.shift_y + rect.y),
                )) * Affine::scale_non_uniform(
                    f64::from(rect.width) / f64::from(image.width),
                    f64::from(rect.height) / f64::from(image.height),
                ),
                brush.as_ref(),
                None,
                &Rect::new(0.0, 0.0, f64::from(image.width), f64::from(image.height)),
            );
        }
        // The boxes of the inline elements on this line go below its text
        // (CSS 2.1 Appendix E: an inline box's background and borders, then
        // its text).
        for piece in &pieces_by_line[line_index] {
            if generated_origin(piece.node)
                .is_some_and(|(_, pseudo)| pseudo == raikiri_style::PseudoElem::Marker)
            {
                continue;
            }
            let retained = root_node.ifc_typographic_fragment(
                piece.node,
                piece.source_container,
                piece.source_owner,
            );
            let Some(cv) = retained
                .map(|(style, _)| style)
                .or_else(|| computed_for_id(cascade, piece.node))
            else {
                continue;
            };
            let source = retained.map(|(_, owner)| owner).unwrap_or(piece.node);
            let (dx, dy) = cumulative_offset(document, root_id, offsets, source);
            // The pieces already carry the line's pagination shift, which
            // `position` holds too.
            let line_shift = shifts.get(line_index).copied().unwrap_or(0.0);
            let group = paint.piece_group(piece);
            crate::walk::paint_inline_box(
                paint.target(group),
                cv,
                piece,
                (!vertical)
                    .then(|| background_slices.get(&piece.node))
                    .flatten(),
                position.x + dx,
                position.y + position.shift_y + dy - line_shift,
                pixel_source,
                warnings,
            );
        }
        if !vertical && let Some(hit_layout) = hit_layout.as_ref() {
            draw_custom_highlights(
                paint.target(None),
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
        let resolved_decorations = raikiri_dom::text_decoration::positioned_line_decorations(
            document,
            cascade,
            root_id,
            base_decorations,
            &positioned_line,
            (position.x, position.y + position.shift_y),
        );
        let mut runs: Vec<RunDraw<'_>> = Vec::new();
        let converter = positioned_line.converter;
        for (positioned_run, decorations) in
            positioned_line.runs.into_iter().zip(resolved_decorations)
        {
            let run = positioned_run.run;
            let cv = positioned_run.style;
            let glyphs: Vec<AnyrenderGlyph> = positioned_run
                .glyphs
                .iter()
                .map(|glyph| AnyrenderGlyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                })
                .collect();
            let offset = positioned_run.offset;
            let group = paint.nearest(positioned_run.style_owner, positioned_run.owner);
            runs.push(RunDraw {
                run,
                style: cv,
                color: css_color_to_peniko(cv.color),
                glyphs,
                decorations,
                offset,
                group,
            });
        }
        if !vertical {
            for draw in &runs {
                draw_decorations(
                    paint.target(draw.group),
                    std::slice::from_ref(draw),
                    DecorationPhase::BeforeGlyphs,
                );
            }
        }
        for draw in &runs {
            let Some(font) = draw.run.font_data() else {
                continue;
            };
            let owner_style = draw.style;
            paint.start_glyphs(draw.group);
            let scene = paint.target(draw.group);
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
            // Emphasis marks sit over or under their characters; vertical
            // runs need rotated placement, which is not drawn yet.
            if !vertical {
                crate::emphasis::draw_marks(
                    scene,
                    document,
                    &draw.run,
                    &draw.glyphs,
                    (
                        position.x + draw.offset.0,
                        position.y
                            + position.shift_y
                            + draw.offset.1
                            + line.block_offset()
                            + draw.run.baseline(),
                    ),
                    owner_style,
                    draw.color,
                );
            }
        }
        if !vertical {
            for draw in &runs {
                draw_decorations(
                    paint.target(draw.group),
                    std::slice::from_ref(draw),
                    DecorationPhase::AfterGlyphs,
                );
            }
        }
        paint.finish(
            scene,
            Rect::new(
                0.0,
                0.0,
                f64::from(page_box.width.max(0.0)),
                f64::from(page_box.height.max(0.0)),
            ),
            paint_transform,
        );
    }
}

#[cfg(test)]
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
    draw_ifc_lines_with_resources(
        scene,
        document,
        cascade,
        root_id,
        position,
        base_decorations,
        fragmentainer,
        custom_highlights,
        None,
        &mut Vec::new(),
        raikiri_traits::PageBox::A4,
        Affine::IDENTITY,
    );
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
        let foreground = cascade
            .computed
            .get(highlight.node)
            .map_or(raikiri_style::CssColor::BLACK, |values| values.color);
        let Some(color) = cascade.custom_highlight_background(&highlight.name, foreground) else {
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
        let color = css_color_to_peniko(color);
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

fn draw_decorations(scene: &mut impl PaintScene, runs: &[RunDraw<'_>], phase: DecorationPhase) {
    for draw in runs {
        draw_resolved_decoration_phase(scene, &draw.decorations, phase);
    }
}

#[cfg(test)]
mod tests;
