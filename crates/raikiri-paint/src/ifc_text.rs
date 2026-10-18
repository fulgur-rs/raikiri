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
use anyrender::{Glyph as AnyrenderGlyph, PaintScene, Scene};
use kurbo::{Affine, Rect};
use peniko::{Fill, Mix};
use raikiri_dom::generated_content::{computed_for_id, generated_origin};
use raikiri_dom::{Document, PositionedLines, cumulative_offset};
use raikiri_style::CascadeResult;
use raikiri_style::property::TextShadowColor;
use shodo::geometry::{PhysicalConverter, WritingMode};
use shodo::hit::{LineLayout, TextPosition};
use shodo::node::NodeId;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::Arc;

mod line_paint;
use line_paint::{LineGroups, LinePaint, PiecesByLine, block_pieces, kept_whole_box};
mod typographic;
pub(crate) use typographic::{BackgroundSlice, Paragraph, enclosing_inline_opacities};

/// Content-box origin of an ifc root in page coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IfcPosition {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) shift_y: f32,
}

/// One glyph run ready to draw, with its decoration context.
/// The glyph `glyph` as the scene draws it.
fn anyrender_glyph(glyph: &raikiri_dom::LineGlyph) -> AnyrenderGlyph {
    AnyrenderGlyph {
        id: glyph.id,
        x: glyph.x,
        y: glyph.y,
    }
}

struct RunDraw<'a> {
    run: shodo::GlyphRunView<'a>,
    style: &'a raikiri_style::ComputedValues,
    color: peniko::Color,
    glyphs: Vec<raikiri_dom::LineGlyph>,
    decorations: Vec<raikiri_dom::DecorationLine>,
    /// The relative offsets of the run's inline ancestors.
    offset: (f32, f32),
    group: Option<usize>,
    /// The box the run's text belongs to: a typographic pseudo's box, or
    /// the parent element of a text node.
    paint_box: usize,
    /// The baseline of a ruby annotation's run, which lies outside the
    /// line's own runs; `None` for a run of the line.
    annotation_baseline: Option<f32>,
    /// The run's glyph axes in the paragraph's logical coordinates.
    glyph_transform: shodo::GlyphTransform,
    /// A ruby annotation run across the paragraph's inline axis, which draws
    /// no emphasis marks.
    turned: bool,
}

/// Draw the glyph runs of an ifc root.
///
/// Runs are drawn in line order. Each run paints its text shadows, then its
/// underlines and overlines, then its glyphs and emphasis marks, then its
/// line-throughs (CSS Text Decoration 3 §3 and §4: shadows below the text
/// and its decorations, underlines and overlines below the text,
/// line-throughs over it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_ifc_lines_with_resources(
    image_cache: &crate::walk::ImageBrushCache,
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
    page_lines: Option<crate::walk::PageLines>,
    paint_transform: Affine,
    fixed_viewport: Option<Rect>,
) {
    if let Some(lines) = hold_ifc_lines(
        image_cache,
        document,
        cascade,
        root_id,
        position,
        base_decorations,
        fragmentainer,
        custom_highlights,
        pixel_source,
        warnings,
        page_box,
        page_lines,
        paint_transform,
        fixed_viewport,
    ) {
        lines.finish(scene);
    }
}

/// The painted lines of a paragraph whose groups are not composited yet, so
/// the boxes laid out beside the lines can still join them.
pub(crate) struct HeldLines<'a> {
    paragraph: Paragraph<'a>,
    rest: typographic::TypographicPaint,
    /// The paint of a translucent first line, with its opacity.
    first_line: Option<(typographic::TypographicPaint, f32)>,
    clip: Rect,
    paint_transform: Affine,
    /// The preorder positions of the paragraph's nodes, once a positioned
    /// box needs its tree order among the groups.
    positions: Option<HashMap<usize, usize>>,
}

impl HeldLines<'_> {
    /// Whether the boxes beside the lines can join a group: the paragraph
    /// has a translucent inline element, typographic pseudo or first line.
    pub(crate) fn has_groups(&self) -> bool {
        self.first_line.is_some() || self.rest.has_groups()
    }

    /// The paragraph root whose lines these are.
    pub(crate) fn root_id(&self) -> usize {
        self.paragraph.root_id
    }

    /// Place the paint of `id`, a box laid out beside the lines, painted in
    /// page space. It joins the groups of the translucent inline elements
    /// around it and, for an atomic inline on the first line, the group of
    /// a translucent `::first-line` (CSS Color 4 §3.3; CSS Pseudo 4
    /// §2.1.1), at the box's stack level (CSS 2.1 Appendix E).
    pub(crate) fn place_box(&mut self, id: usize, content: Scene) {
        let paragraph = self.paragraph;
        let on_first_line = self.first_line.is_some()
            && raikiri_dom::paint_rules::atomic_on_first_line(
                paragraph.document,
                paragraph.root_id,
                id,
            );
        let paint = match self.first_line.as_mut() {
            Some((first, _)) if on_first_line => first,
            _ => &mut self.rest,
        };
        let group = paragraph
            .document
            .parent_of(id)
            .and_then(|parent| paint.element_group(paragraph, parent));
        let positioned = computed_for_id(paragraph.cascade, id)
            .filter(|cv| cv.position != raikiri_style::property::PositionValue::Static)
            .map(|_| {
                let positions = self.positions.get_or_insert_with(|| {
                    paragraph.document.preorder_positions(paragraph.root_id)
                });
                let at = positions.get(&id).map_or((usize::MAX, 0), |&at| (at, 0));
                let before = paint.groups_before(|owner| {
                    paragraph
                        .document
                        .group_tree_position(positions, owner)
                        .is_some_and(|group_at| group_at < at)
                });
                (
                    typographic::element_stack_level(paragraph.cascade, id),
                    before,
                )
            });
        // The groups are recorded before the paragraph's paint transform,
        // which composites them; the box was painted after it.
        let inverse = self.paint_transform.inverse();
        if !inverse.as_coeffs().iter().all(|value| value.is_finite()) {
            // A singular transform leaves no two-dimensional ink to place.
            return;
        }
        let mut local = Scene::new();
        local.append_scene(content, inverse);
        paint.add_box(group, positioned, local);
    }

    /// Place the paint of an in-flow block inside the paragraph's inline
    /// content, painted in page space, above the content positioned below
    /// the paragraph's blocks and below the rest of the lines.
    pub(crate) fn place_block(&mut self, content: Scene) {
        let inverse = self.paint_transform.inverse();
        // A singular transform leaves no two-dimensional ink to place.
        if inverse.as_coeffs().iter().all(|value| value.is_finite()) {
            let mut local = Scene::new();
            local.append_scene(content, inverse);
            self.rest.add_block(local);
        }
    }

    /// The transform the lines were painted in, which [`Self::finish`]
    /// expects on `scene`.
    pub(crate) fn paint_transform(&self) -> Affine {
        self.paint_transform
    }

    /// Composite the paragraph's groups into `scene`.
    pub(crate) fn finish(self, scene: &mut impl PaintScene) {
        self.rest
            .finish(scene, self.clip, self.paint_transform, self.first_line);
    }

    /// Composite the paragraph's groups like [`Self::finish`], apart from
    /// its blocks: the paint below the blocks, the blocks in the order they
    /// were placed, and the paint above them. When `leaving`, the groups of
    /// the positioned inline elements that form a stacking context in the
    /// paragraph's stacking context (see
    /// [`raikiri_dom::paint_rules::leaves_paragraph_content`]) are left out
    /// too, and returned by element, in page space, for the caller to paint
    /// among the positioned boxes of that stacking context.
    pub(crate) fn finish_leaving_groups(
        self,
        leaving: bool,
    ) -> (Scene, Vec<Scene>, Scene, Vec<(usize, Scene)>) {
        let paragraph = self.paragraph;
        let leaving = leaving
            && raikiri_dom::paint_rules::content_can_leave_paragraph(
                paragraph.document,
                paragraph.cascade,
                paragraph.root_id,
            );
        let leaves = |owner: usize| {
            raikiri_dom::paint_rules::leaves_paragraph_content(
                paragraph.document,
                paragraph.cascade,
                paragraph.root_id,
                owner,
                true,
            )
        };
        let paint_transform = self.paint_transform;
        let (below, blocks, above, left) = self.rest.finish_leaving(
            self.clip,
            paint_transform,
            self.first_line,
            leaving.then_some(&leaves as &dyn Fn(usize) -> bool),
        );
        let left = left
            .into_iter()
            .map(|(owner, content)| {
                let mut placed = Scene::new();
                placed.append_scene(content, paint_transform);
                (owner, placed)
            })
            .collect();
        (below, blocks, above, left)
    }
}

/// Paint the lines of `root_id` like [`draw_ifc_lines_with_resources`], but
/// hold their groups open; `None` when the paragraph has no lines.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hold_ifc_lines<'a>(
    image_cache: &crate::walk::ImageBrushCache,
    document: &'a Document,
    cascade: &'a CascadeResult,
    root_id: usize,
    position: IfcPosition,
    base_decorations: &DecorationContext,
    fragmentainer: Option<usize>,
    custom_highlights: &[crate::TextHighlightRange],
    pixel_source: Option<&dyn raikiri_traits::ImagePixelSource>,
    warnings: &mut Vec<raikiri_traits::RenderWarning>,
    page_box: raikiri_traits::PageBox,
    page_lines: Option<crate::walk::PageLines>,
    paint_transform: Affine,
    fixed_viewport: Option<Rect>,
) -> Option<HeldLines<'a>> {
    let positioned = PositionedLines::new(document, cascade, root_id, fragmentainer)?;
    let writing_mode = positioned.writing_mode;
    let shifts = &positioned.shifts;
    let base = position;
    let size = positioned.size;
    let vertical = writing_mode != WritingMode::HorizontalTb;
    // Pages never cut a paragraph across its lines: each of its lines goes
    // with the paragraph's content box.
    let paragraph_box = kept_whole_box(document, root_id, page_lines, vertical, base);
    let pieces = PiecesByLine::new(&positioned, document, cascade, root_id);
    let offsets = positioned.offsets;
    let hit_layout =
        (!custom_highlights.is_empty()).then(|| LineLayout::new(positioned.all_lines()));
    let root_node = document
        .ifc_layout_node(root_id)
        .expect("positioned lines require an existing IFC root");
    let paragraph = typographic::Paragraph {
        document,
        cascade,
        root_id,
    };
    // A page holds a paragraph it keeps whole as it holds its lines.
    let off_band = page_lines
        .zip(paragraph_box)
        .is_some_and(|(lines, rect)| !lines.holds_line(paint_transform, rect));
    let blocks = block_pieces(root_node, off_band);
    let block_nodes: std::collections::HashSet<usize> =
        blocks.iter().map(|(piece, _)| piece.node).collect();
    let mut groups = LineGroups::new(paragraph, root_node, &pieces.pieces, &blocks);
    let mut lines = LinePaint {
        image_cache,
        paragraph,
        offsets,
        base,
        writing_mode,
        shifts,
        root_node,
        pieces,
        block_nodes,
        paint_transform,
        fixed_viewport,
        pixel_source,
        warnings,
    };
    lines.paint_block_pieces(&mut groups, &blocks);
    for positioned_line in positioned.lines() {
        let line_index = positioned_line.index;
        let paint = groups.for_line(line_index);
        let line = positioned_line.line;
        let (column_x, column_y) = positioned_line.offset;
        let position = IfcPosition {
            x: base.x + column_x,
            y: base.y + column_y,
            shift_y: base.shift_y,
        };
        if let Some(lines) = page_lines
            && !lines.holds_line(
                paint_transform,
                paragraph_box.unwrap_or_else(|| line_rect(&positioned_line, position)),
            )
        {
            continue;
        }
        let transform = Affine::translate((
            f64::from(position.x),
            f64::from(position.y + position.shift_y),
        ));
        lines.paint_marker_images(paint, &positioned_line.markers, position);
        lines.paint_line_boxes(paint, line_index, position);
        lines.paint_content_images(paint, &positioned_line.content_images, line_index, position);
        if !vertical && let Some(hit_layout) = hit_layout.as_ref() {
            draw_custom_highlights(
                paint,
                paragraph,
                line_index,
                line,
                hit_layout,
                position,
                offsets,
                custom_highlights,
                &positioned_line.runs,
                &positioned_line.converter,
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
        for (mut positioned_run, decorations) in
            positioned_line.runs.into_iter().zip(resolved_decorations)
        {
            let run = positioned_run.run;
            let cv = positioned_run.style;
            let glyphs = std::mem::take(&mut positioned_run.glyphs);
            let offset = positioned_run.offset;
            let paint_box = paint_box(
                document,
                cascade,
                positioned_run.owner,
                positioned_run.style_owner,
            );
            // Text outside every typographic box is in the groups of the
            // inline elements around it.
            let group = paint
                .nearest(positioned_run.style_owner, positioned_run.owner)
                .or_else(|| paint.element_group(paragraph, paint_box));
            runs.push(RunDraw {
                run,
                glyph_transform: positioned_run.glyph_transform(),
                turned: positioned_run
                    .annotation
                    .is_some_and(|annotation| annotation.turned),
                style: cv,
                color: css_color_to_peniko(cv.color),
                glyphs,
                decorations,
                offset,
                group,
                paint_box,
                annotation_baseline: positioned_run
                    .annotation
                    .map(|annotation| annotation.baseline),
            });
        }
        // Consecutive runs of one box paint in phases: every shadow, then
        // every underline and overline, then the glyphs, then every
        // line-through (CSS 2.2 Appendix E; CSS Text Decoration 3 §4).
        // Different boxes keep their tree order, so a later box's shadow
        // still covers an earlier box's text.
        for segment in runs.chunk_by(|a, b| a.paint_box == b.paint_box) {
            if !vertical {
                // Each shadow layer covers every run of the box before the
                // next one up, so the first declared shadow stays on top.
                let layers = segment
                    .iter()
                    .map(|draw| draw.style.text_shadow.len())
                    .max()
                    .unwrap_or(0);
                for layer in (0..layers).rev() {
                    for draw in segment {
                        let Some(font) = draw.run.font_data() else {
                            continue;
                        };
                        let Some(shadow) = draw.style.text_shadow.get(layer) else {
                            continue;
                        };
                        let coords = normalized_coords(&draw.run);
                        let glyph_transform = physical_glyph_transform(
                            &converter,
                            draw.glyph_transform,
                            draw.run.skew(),
                        );
                        draw_shadow(
                            paint.target(draw.group),
                            &ShadowRun {
                                draw,
                                font: &font,
                                coords: &coords,
                                glyph_transform,
                                document,
                                missing_glyph_boxes: !vertical && !draw.turned,
                                emphasis_origin: emphasis_origin(
                                    position,
                                    line.block_offset(),
                                    draw,
                                ),
                            },
                            shadow,
                            draw.style,
                            run_transform(transform, draw),
                            IfcPosition {
                                x: position.x + draw.offset.0,
                                y: position.y + draw.offset.1,
                                shift_y: position.shift_y,
                            },
                            size,
                        );
                    }
                }
                for draw in segment {
                    draw_resolved_decoration_phase(
                        paint.target(draw.group),
                        &draw.decorations,
                        DecorationPhase::BeforeGlyphs,
                    );
                }
            }
            for draw in segment {
                let Some(font) = draw.run.font_data() else {
                    continue;
                };
                let owner_style = draw.style;
                let font_size = draw.run.font_size();
                let coords = normalized_coords(&draw.run);
                let glyph_transform =
                    physical_glyph_transform(&converter, draw.glyph_transform, draw.run.skew());
                let run_transform = run_transform(transform, draw);
                let scene = paint.target(draw.group);
                if crate::text::has_glyph_outlines(&font) {
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
                        draw.glyphs.iter().map(anyrender_glyph),
                    );
                } else if !vertical && !draw.turned {
                    // Horizontal runs only: a vertical run's advances run
                    // down the line, which the boxes do not follow yet.
                    crate::text::draw_missing_glyph_boxes(
                        scene,
                        crate::text::missing_glyph_origins(
                            &draw.run,
                            draw.glyphs
                                .iter()
                                .map(|glyph| (glyph.cluster, glyph.x, glyph.y)),
                        ),
                        font_size,
                        draw.color,
                        run_transform,
                    );
                }
                // Emphasis marks sit over or under their characters. A
                // vertical annotation run's clusters are laid out on its own
                // line, whose inline axis the marks are not placed along yet.
                if !vertical && !draw.turned {
                    crate::emphasis::draw_marks(
                        scene,
                        document,
                        &draw.run,
                        &draw.glyphs,
                        emphasis_origin(position, line.block_offset(), draw),
                        owner_style,
                        draw.color,
                    );
                } else if vertical && draw.annotation_baseline.is_none() {
                    crate::emphasis::draw_vertical_marks(
                        scene,
                        document,
                        (line, &draw.run),
                        (writing_mode, &converter),
                        (
                            position.x + draw.offset.0,
                            position.y + position.shift_y + draw.offset.1,
                        ),
                        owner_style,
                        draw.color,
                    );
                }
            }
            if !vertical {
                for draw in segment {
                    draw_resolved_decoration_phase(
                        paint.target(draw.group),
                        &draw.decorations,
                        DecorationPhase::AfterGlyphs,
                    );
                }
            }
        }
    }
    Some(HeldLines {
        paragraph,
        rest: groups.rest,
        first_line: groups.first_line,
        clip: Rect::new(
            0.0,
            0.0,
            f64::from(page_box.width.max(0.0)),
            f64::from(page_box.height.max(0.0)),
        ),
        paint_transform,
        positions: None,
    })
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
        &crate::walk::ImageBrushCache::for_document(document)
            .with_device_scale(cascade.border_width_device_scale()),
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
        None,
        Affine::IDENTITY,
        None,
    );
}

/// The page area in which the `fixed` background layers of the inline box
/// `piece`, styled `cv`, are positioned, given the page area `fixed_viewport`
/// that the layers of its paragraph's block are positioned in. A box without
/// a `fixed` layer needs none. Inside an inline element that is the
/// containing block of fixed-position boxes, a `fixed` layer is positioned
/// like a `scroll` one, so the box gets no page area.
fn piece_fixed_viewport(
    paragraph: Paragraph<'_>,
    cv: &raikiri_style::ComputedValues,
    piece: &raikiri_dom::InlineBoxPiece,
    fixed_viewport: Option<Rect>,
) -> Option<Rect> {
    fixed_viewport.filter(|_| {
        cv.background_layers()
            .iter()
            .any(|layer| layer.attachment == raikiri_style::property::BackgroundAttachment::Fixed)
            && !typographic::in_fixed_containing_block(paragraph, piece)
    })
}

/// Where `line` paints, before the transforms around its paragraph.
fn line_rect(line: &raikiri_dom::PositionedLine<'_>, position: IfcPosition) -> kurbo::Rect {
    let rect = line.converter.rect(shodo::geometry::LogicalRect {
        inline_start: 0.0,
        block_start: line.line.block_offset(),
        inline_size: line.line.inline_size(),
        block_size: line.line.block_size(),
    });
    kurbo::Rect::from_origin_size(
        (
            f64::from(position.x + rect.x),
            f64::from(position.y + position.shift_y + rect.y),
        ),
        (f64::from(rect.width), f64::from(rect.height)),
    )
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
    let transform = orientation * crate::text::synthetic_slant(skew);
    (transform != Affine::IDENTITY).then_some(transform)
}

#[allow(clippy::too_many_arguments)]
fn draw_custom_highlights(
    paint: &mut typographic::TypographicPaint,
    paragraph: Paragraph<'_>,
    line_index: usize,
    line: &shodo::Line,
    hit_layout: &LineLayout<'_>,
    position: IfcPosition,
    offsets: &[(usize, (f32, f32))],
    custom_highlights: &[crate::TextHighlightRange],
    runs: &[raikiri_dom::PositionedRun<'_>],
    converter: &PhysicalConverter,
) {
    let highlights = LineHighlights {
        paragraph,
        position,
        offsets,
        custom_highlights,
        converter,
    };
    draw_line_highlights(
        paint,
        &highlights,
        line,
        hit_layout,
        line_index,
        |inline, block| (inline, block),
    );
    // A ruby annotation's text lies on a line of its own, which the
    // paragraph's line layout does not hold.
    let mut annotations: Vec<raikiri_dom::AnnotationRun<'_>> = Vec::new();
    for annotation in runs.iter().filter_map(|run| run.annotation) {
        // An annotation with several runs is drawn once.
        if !annotations
            .iter()
            .any(|seen| std::ptr::eq(seen.line, annotation.line))
        {
            annotations.push(annotation);
        }
    }
    for annotation in annotations {
        let layout = LineLayout::new(std::slice::from_ref(annotation.line));
        draw_line_highlights(
            paint,
            &highlights,
            annotation.line,
            &layout,
            0,
            |inline, block| annotation.to_paragraph(inline, block),
        );
    }
}

/// The custom highlights of one line of a paragraph and where they paint.
struct LineHighlights<'a> {
    paragraph: Paragraph<'a>,
    position: IfcPosition,
    offsets: &'a [(usize, (f32, f32))],
    custom_highlights: &'a [crate::TextHighlightRange],
    /// The map from the paragraph's logical coordinates to physical ones.
    converter: &'a PhysicalConverter,
}

/// Fill the highlighted text of `line`, line `line_index` of
/// `hit_layout`, whose logical coordinates `place` maps to the
/// paragraph's.
fn draw_line_highlights(
    paint: &mut typographic::TypographicPaint,
    highlights: &LineHighlights<'_>,
    line: &shodo::Line,
    hit_layout: &LineLayout<'_>,
    line_index: usize,
    place: impl Fn(f32, f32) -> (f32, f32),
) {
    let Paragraph {
        document,
        cascade,
        root_id,
    } = highlights.paragraph;
    let position = highlights.position;
    let line_range = line.text_range();
    let Some(offset_mapping) = line.offset_mapping() else {
        return;
    };
    for highlight in highlights.custom_highlights {
        let values = cascade.computed.get(highlight.node);
        // Hidden text is laid out but not painted, and neither is its
        // highlight. Its line can still hold visible text.
        if values
            .is_some_and(|values| values.visibility != raikiri_style::property::Visibility::Visible)
        {
            continue;
        }
        let foreground = values.map_or(raikiri_style::CssColor::BLACK, |values| values.color);
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
        let (offset_x, offset_y) =
            cumulative_offset(document, root_id, highlights.offsets, highlight.node);
        // The highlight paints with its text, in the groups of the inline
        // elements around it.
        let group = paint.element_group(
            highlights.paragraph,
            paint_box(document, cascade, highlight.node, highlight.node),
        );
        let scene = paint.target(group);
        for selected in selection {
            let (inline_a, block_a) = place(selected.inline_start, selected.block_start);
            let (inline_b, block_b) = place(
                selected.inline_start + selected.inline_size,
                selected.block_start + selected.block_size,
            );
            let placed = highlights.converter.rect(shodo::geometry::LogicalRect {
                inline_start: inline_a.min(inline_b),
                block_start: block_a.min(block_b),
                inline_size: (inline_b - inline_a).abs(),
                block_size: (block_b - block_a).abs(),
            });
            let x = position.x + offset_x + placed.x;
            let y = position.y + position.shift_y + offset_y + placed.y;
            let rect = Rect::new(
                f64::from(x),
                f64::from(y),
                f64::from(x + placed.width),
                f64::from(y + placed.height),
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
        }
    }
}

/// shodo's normalized coordinates are `F2Dot14` newtypes; the scene takes
/// the raw `i16` bits.
/// The box a run's text is painted as part of: a typographic pseudo's box,
/// or else the nearest element above the text node that generates a box.
/// A `display: contents` element generates none, so its text belongs to the
/// box of its nearest boxed ancestor (CSS Display 3 §2.5).
fn paint_box(
    document: &Document,
    cascade: &CascadeResult,
    owner: usize,
    style_owner: usize,
) -> usize {
    // A typographic pseudo's run, and a `text-overflow` ellipsis (owned by
    // the paragraph itself), already name their box.
    let is_text = document
        .get_node(owner)
        .is_some_and(|node| node.text_content().is_some());
    if style_owner != owner || !is_text {
        return style_owner;
    }
    // Past the last ancestor, the walk stays on it even when it is
    // `display: contents`.
    let mut node = owner;
    for id in document.ancestors(owner) {
        node = id;
        if !computed_for_id(cascade, id)
            .is_some_and(|cv| cv.display == raikiri_style::property::DisplayValue::Contents)
        {
            break;
        }
    }
    node
}

fn normalized_coords(run: &shodo::GlyphRunView<'_>) -> Vec<i16> {
    run.normalized_coords()
        .iter()
        .map(|coord| coord.to_bits())
        .collect()
}

/// The line transform moved by the relative offsets of the run's inline
/// ancestors.
fn run_transform(line: Affine, draw: &RunDraw<'_>) -> Affine {
    line * Affine::translate((f64::from(draw.offset.0), f64::from(draw.offset.1)))
}

/// A glyph run with the font data the scene needs to draw it again.
struct ShadowRun<'a, 'b> {
    draw: &'a RunDraw<'b>,
    font: &'a peniko::FontData,
    coords: &'a [i16],
    glyph_transform: Option<Affine>,
    document: &'a Document,
    /// Where the run's emphasis marks are placed from: its start and its
    /// baseline.
    emphasis_origin: (f32, f32),
    /// The run draws missing-glyph boxes where its font has no outlines.
    missing_glyph_boxes: bool,
}

/// The start and baseline a run's emphasis marks are placed from.
fn emphasis_origin(position: IfcPosition, block_offset: f32, draw: &RunDraw<'_>) -> (f32, f32) {
    (
        position.x + draw.offset.0,
        position.y
            + position.shift_y
            + draw.offset.1
            + draw
                .annotation_baseline
                .unwrap_or(block_offset + draw.run.baseline()),
    )
}

/// Draw one text shadow of one run, behind its glyphs and decorations.
///
/// CSS Text Decoration 3 §4 paints the shadows below the text, the first one
/// on top, so callers draw the list in reverse. Each shadow is the run's glyphs
/// and its decoration lines, in their paint order, filled with the shadow
/// color. A blurred shadow is drawn inside a filter layer clipped to the
/// lines and the run's glyphs, grown by three times the blur radius, which
/// isolates the blur from the rest of the page. The Gaussian's standard deviation is half the blur
/// radius (CSS Backgrounds 3 §7.1).
fn draw_shadow(
    scene: &mut impl PaintScene,
    run: &ShadowRun<'_, '_>,
    shadow: &raikiri_style::ComputedTextShadow,
    owner_cv: &raikiri_style::ComputedValues,
    transform: Affine,
    position: IfcPosition,
    (width, height): (f32, f32),
) {
    let font_size = run.draw.run.font_size();
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
        // Text that overflows its paragraph (`nowrap`, a negative
        // `text-indent`) lies outside the paragraph's own width, and its
        // shadow with it, so the clip also covers the run's glyphs. A glyph's
        // ink can stick out of its advance box (an italic or synthetic
        // oblique overhang, a mark, a swash, tight letter spacing), so each
        // glyph is padded by one em, which the ink of an ordinary glyph does
        // not exceed.
        let pad = f64::from(font_size);
        let (ink_left, ink_right) =
            run.draw
                .glyphs
                .iter()
                .fold((0.0_f64, f64::from(width)), |(left, right), glyph| {
                    let (start, end) = (f64::from(glyph.x), f64::from(glyph.x + glyph.advance));
                    (
                        left.min(start.min(end) - pad),
                        right.max(start.max(end) + pad),
                    )
                });
        let clip = Rect::new(
            x + ink_left + dx - extent,
            y + dy - extent,
            x + ink_right + dx + extent,
            y + f64::from(height) + dy + extent,
        );
        scene.push_layer(
            Mix::Normal,
            1.0,
            Affine::IDENTITY,
            &clip,
            Some(Arc::new(Filter::single(FilterEffect::blur(blur / 2.0)))),
            None,
        );
    }
    let decorations: Vec<_> = run
        .draw
        .decorations
        .iter()
        .map(|line| {
            let mut line = line.clone();
            let (dx, dy) = (dx as f32, dy as f32);
            line.x_start += dx;
            line.x_end += dx;
            line.pattern_origin_x += dx;
            line.pattern_end_x += dx;
            line.y += dy;
            line.color = color;
            line
        })
        .collect();
    draw_resolved_decoration_phase(scene, &decorations, DecorationPhase::BeforeGlyphs);
    if crate::text::has_glyph_outlines(run.font) {
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
            run.draw.glyphs.iter().map(anyrender_glyph),
        );
    } else if run.missing_glyph_boxes {
        crate::text::draw_missing_glyph_boxes(
            scene,
            crate::text::missing_glyph_origins(
                &run.draw.run,
                run.draw
                    .glyphs
                    .iter()
                    .map(|glyph| (glyph.cluster, glyph.x, glyph.y)),
            ),
            font_size,
            css_color_to_peniko(color),
            shadow_transform,
        );
    }
    let (x, baseline) = run.emphasis_origin;
    crate::emphasis::draw_marks_in(
        scene,
        run.document,
        &run.draw.run,
        &run.draw.glyphs,
        (x + dx as f32, baseline + dy as f32),
        owner_cv,
        css_color_to_peniko(color),
    );
    draw_resolved_decoration_phase(scene, &decorations, DecorationPhase::AfterGlyphs);
    if use_blur_layer {
        scene.pop_layer();
    }
}

#[cfg(test)]
mod tests;

/// Whether the box of the element or pseudo-element `id` that an inline
/// piece paints has a transform that applies to it: an atomic `::before` or
/// `::after` with a transform, or a transformable element with one.
fn transform_applies(
    document: &Document,
    cascade: &CascadeResult,
    id: usize,
    cv: &raikiri_style::ComputedValues,
) -> bool {
    if generated_origin(id).is_some() {
        raikiri_dom::paint_rules::pseudo_transform(cascade, id).is_some()
    } else {
        document.get_node(id).is_some() && raikiri_dom::has_applied_transform(document, id, cv)
    }
}
