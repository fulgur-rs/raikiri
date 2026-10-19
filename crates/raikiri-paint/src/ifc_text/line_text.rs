//! The text of a line of a paragraph: the runs with their shadows,
//! decorations, glyphs and emphasis marks.

use super::line_paint::LinePaint;
use super::typographic::{Paragraph, TypographicPaint};
use super::{
    IfcPosition, RunDraw, ShadowMarks, ShadowPiece, ShadowRun, VerticalMarks, anyrender_glyph,
    draw_shadow, emphasis_origin, normalized_coords, paint_box, physical_glyph_transform,
    run_transform,
};
use crate::text::{
    DecorationPhase, css_color_to_peniko, draw_resolved_decoration_phase, synthetic_embolden,
};
use anyrender::PaintScene;
use kurbo::Affine;
use peniko::Fill;
use raikiri_dom::{DecorationLine, PositionedLine, PositionedRun};
use shodo::geometry::PhysicalConverter;

/// Where a line paints.
struct LineFrame<'l> {
    line: &'l shodo::Line,
    converter: PhysicalConverter,
    position: IfcPosition,
    /// The map from the line's origin to the page.
    transform: Affine,
}

impl<'a> LinePaint<'a, '_> {
    /// Paint the runs of `line`, which sits at `position`.
    pub(super) fn paint_text(
        &self,
        paint: &mut TypographicPaint,
        line: PositionedLine<'a>,
        position: IfcPosition,
    ) {
        let Paragraph {
            document,
            cascade,
            root_id,
        } = self.paragraph;
        let transform = Affine::translate((
            f64::from(position.x),
            f64::from(position.y + position.shift_y),
        ));
        let resolved_decorations = raikiri_dom::text_decoration::positioned_line_decorations(
            document,
            cascade,
            root_id,
            self.base_decorations,
            &line,
            (position.x, position.y + position.shift_y),
        );
        let frame = LineFrame {
            line: line.line,
            converter: line.converter,
            position,
            transform,
        };
        let runs = self.run_draws(paint, line.runs, resolved_decorations);
        // Consecutive runs of one box paint in phases: every shadow, then
        // every underline and overline, then the glyphs, then every
        // line-through (CSS 2.2 Appendix E; CSS Text Decoration 3 §4).
        // Different boxes keep their tree order, so a later box's shadow
        // still covers an earlier box's text.
        for segment in runs.chunk_by(|a, b| a.paint_box == b.paint_box) {
            self.paint_shadows(paint, segment, &frame);
            if !self.vertical {
                paint_decorations(paint, segment, DecorationPhase::BeforeGlyphs);
            }
            self.paint_glyphs(paint, segment, &frame);
            if !self.vertical {
                paint_decorations(paint, segment, DecorationPhase::AfterGlyphs);
            }
        }
    }

    /// The runs of a line with what each is drawn with, in line order.
    fn run_draws(
        &self,
        paint: &mut TypographicPaint,
        runs: Vec<PositionedRun<'a>>,
        decorations: Vec<Vec<DecorationLine>>,
    ) -> Vec<RunDraw<'a>> {
        let Paragraph {
            document, cascade, ..
        } = self.paragraph;
        let mut draws = Vec::new();
        for (mut positioned_run, decorations) in runs.into_iter().zip(decorations) {
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
                .or_else(|| paint.element_group(self.paragraph, paint_box));
            draws.push(RunDraw {
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
        draws
    }

    /// Paint the text shadows of the runs of one box.
    ///
    /// Each shadow layer covers every run of the box before the next one up,
    /// so the first declared shadow stays on top. The runs of a layer that
    /// paint into one group with one blur radius are blurred together.
    fn paint_shadows(
        &self,
        paint: &mut TypographicPaint,
        segment: &[RunDraw<'_>],
        frame: &LineFrame<'_>,
    ) {
        let position = frame.position;
        let layers = segment
            .iter()
            .map(|draw| draw.style.text_shadow.len())
            .max()
            .unwrap_or(0);
        if layers == 0 {
            return;
        }
        let runs: Vec<_> = runs_with_fonts(segment)
            .map(|(draw, font)| {
                let coords = normalized_coords(&draw.run);
                let glyph_transform = physical_glyph_transform(
                    &frame.converter,
                    draw.glyph_transform,
                    draw.run.skew(),
                );
                (draw, font, coords, glyph_transform)
            })
            .collect();
        for layer in (0..layers).rev() {
            let pieces: Vec<ShadowPiece<'_, '_>> = runs
                .iter()
                .filter_map(|(draw, font, coords, glyph_transform)| {
                    let shadow = draw.style.text_shadow.get(layer)?;
                    Some(ShadowPiece {
                        run: ShadowRun {
                            draw,
                            font,
                            coords,
                            glyph_transform: *glyph_transform,
                            document: self.paragraph.document,
                            marks: self.shadow_marks(frame, draw),
                            vertical: self.vertical,
                            missing_glyph_boxes: !self.vertical && !draw.turned,
                        },
                        shadow,
                        transform: run_transform(frame.transform, draw),
                        position: IfcPosition {
                            x: position.x + draw.offset.0,
                            y: position.y + draw.offset.1,
                            shift_y: position.shift_y,
                        },
                    })
                })
                .collect();
            for together in
                pieces.chunk_by(|a, b| a.run.draw.group == b.run.draw.group && a.blur() == b.blur())
            {
                draw_shadow(paint.target(together[0].run.draw.group), together);
            }
        }
    }

    /// Where the emphasis marks of `draw`'s shadows are placed from.
    ///
    /// A run draws its shadows' marks where it draws its own: none for a run
    /// of a ruby annotation in a vertical paragraph, whose clusters are laid
    /// out on its own line.
    fn shadow_marks<'f>(&self, frame: &'f LineFrame<'_>, draw: &RunDraw<'_>) -> ShadowMarks<'f> {
        let position = frame.position;
        if !self.vertical {
            return ShadowMarks::Horizontal(emphasis_origin(
                position,
                frame.line.block_offset(),
                draw,
            ));
        }
        if draw.annotation_baseline.is_some() {
            return ShadowMarks::None;
        }
        ShadowMarks::Vertical(VerticalMarks {
            line: frame.line,
            writing_mode: self.writing_mode,
            converter: &frame.converter,
            origin: (
                position.x + draw.offset.0,
                position.y + position.shift_y + draw.offset.1,
            ),
        })
    }

    /// Paint the glyphs of the runs of one box, with their emphasis marks.
    fn paint_glyphs(
        &self,
        paint: &mut TypographicPaint,
        segment: &[RunDraw<'_>],
        frame: &LineFrame<'_>,
    ) {
        let document = self.paragraph.document;
        let position = frame.position;
        let vertical = self.vertical;
        for (draw, font) in runs_with_fonts(segment) {
            let owner_style = draw.style;
            let font_size = draw.run.font_size();
            let coords = normalized_coords(&draw.run);
            let glyph_transform =
                physical_glyph_transform(&frame.converter, draw.glyph_transform, draw.run.skew());
            let run_transform = run_transform(frame.transform, draw);
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
                    emphasis_origin(position, frame.line.block_offset(), draw),
                    owner_style,
                    draw.color,
                );
            } else if vertical && draw.annotation_baseline.is_none() {
                crate::emphasis::draw_vertical_marks(
                    scene,
                    document,
                    (frame.line, &draw.run),
                    (self.writing_mode, &frame.converter),
                    (
                        position.x + draw.offset.0,
                        position.y + position.shift_y + draw.offset.1,
                    ),
                    owner_style,
                    draw.color,
                );
            }
        }
    }
}

/// Paint the decoration lines of the runs of one box that belong to `phase`.
fn paint_decorations(
    paint: &mut TypographicPaint,
    segment: &[RunDraw<'_>],
    phase: DecorationPhase,
) {
    for draw in segment {
        draw_resolved_decoration_phase(paint.target(draw.group), &draw.decorations, phase);
    }
}

/// The runs of one box with their font data. Positioned lines list only the
/// runs that have a font.
fn runs_with_fonts<'s, 'r>(
    segment: &'s [RunDraw<'r>],
) -> impl Iterator<Item = (&'s RunDraw<'r>, peniko::FontData)> {
    segment
        .iter()
        .filter_map(|draw| draw.run.font_data().map(|font| (draw, font)))
}
