//! The frames that paint the lines of a paragraph: an anonymous table cell's
//! and a block's own.

use super::frames::{
    AnonymousParagraphFrame, LinesFrame, finish_lines_leaving_groups, leave_groups, leaves_groups,
};
use super::paint_walk::PaintWalk;
use super::{
    box_intersects_page, ifc_paint_extent, in_left_group, paint_element_border, traced_text,
};
use crate::ifc_text::{HeldLines, IfcPosition};
use anyrender::PaintScene;
use kurbo::Affine;
use raikiri_dom::paint_rules;
use raikiri_traits::NodeKind;

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Paint the lines of the paragraph of an anonymous table cell.
    pub(super) fn anonymous_paragraph(&mut self, paragraph: AnonymousParagraphFrame<'w>) {
        let AnonymousParagraphFrame {
            key,
            cell,
            x,
            y,
            shift_y,
            transform_x,
            transform_y,
            paint_offset_y,
            fragmentainer,
            inside_fixed,
            decorations,
            fixed_viewport,
            defer: _,
        } = paragraph;
        let (document, cascade) = (self.document, self.cascade);
        let layout = cell.unrounded_layout;
        let probe = cell
            .ifc_lines()
            .and_then(|lines| {
                lines
                    .iter()
                    .flat_map(|line| line.fragments())
                    .find_map(|fragment| {
                        let shodo::Fragment::GlyphRun(run) = fragment else {
                            return None;
                        };
                        let owner = run.node()?.0 as usize;
                        document
                            .get_node(owner)
                            .filter(|node| node.kind() == NodeKind::Text)
                            .map(|_| owner)
                    })
            })
            .unwrap_or(document.ifc_source_owner(key));
        if !paint_rules::named_page_matches(document, cascade, probe, self.active_page_name)
            || (!box_intersects_page(
                y,
                ifc_paint_extent(cell, &layout),
                self.geometry.page_top,
                self.geometry.page_bottom,
            ) && !inside_fixed)
        {
            return;
        }
        let clip = self
            .geometry
            .text_page_clip(cascade, self.page_box, inside_fixed);
        if let Some(clip) = &clip {
            self.scene
                .scene
                .push_clip_layer(self.geometry.page_clip_transform, clip);
        }
        let owner = document.ifc_source_owner(key);
        let cell_on_band = inside_fixed
            || self.geometry.flow_band.is_none_or(|band| {
                band.holds_box(
                    self.scene.transform,
                    (
                        x + self.geometry.page_offset_x + transform_x,
                        y + self.geometry.page_offset_y + transform_y,
                        layout.size.width,
                        layout.size.height,
                    ),
                )
            });
        if !paint_rules::hides_own_box(&cascade.computed[owner])
            && cell_on_band
            && let Some(border) = cell.collapsed_border()
        {
            paint_element_border(
                self.scene,
                layout.size.width,
                layout.size.height,
                x + self.geometry.page_offset_x + transform_x,
                y + self.geometry.page_offset_y + transform_y,
                border,
                cascade.computed[owner].color,
            );
        }
        if let Some(events) = self.trace.as_deref_mut()
            && raikiri_dom::PositionedLines::new(document, cascade, key, fragmentainer)
                .is_some_and(|lines| lines.lines().any(|line| !line.runs.is_empty()))
        {
            events.push(crate::PaintTraceEvent::Text(document.ifc_source_owner(key)));
        }
        let paint_transform = self.scene.transform;
        crate::ifc_text::draw_ifc_lines_with_resources(
            self.image_cache,
            self.scene,
            document,
            cascade,
            key,
            IfcPosition {
                x: x + self.geometry.page_offset_x
                    + transform_x
                    + layout.border.left
                    + layout.padding.left,
                y: y + self.geometry.page_offset_y
                    + transform_y
                    + layout.border.top
                    + layout.padding.top,
                shift_y,
            },
            &decorations,
            fragmentainer,
            self.custom_highlights,
            self.pixel_source,
            self.warnings,
            self.page_box,
            self.geometry
                .page_lines
                .filter(|_| !inside_fixed)
                .map(|lines| lines.translated(transform_y + paint_offset_y)),
            paint_transform,
            fixed_viewport,
        );
        if clip.is_some() {
            self.scene.pop_layer();
        }
    }

    /// Paint the lines of a paragraph, hold them for the boxes laid out beside
    /// them, or set them aside around its blocks.
    pub(super) fn lines(&mut self, frame: LinesFrame) {
        let LinesFrame {
            node_id,
            inside_fixed,
            paint_transform,
            hold,
            boxes_in_groups,
            ..
        } = frame;
        let (document, cascade) = (self.document, self.cascade);
        let early_start = self
            .early_boxes
            .iter()
            .rposition(|&(root, _, _)| root != node_id)
            .map_or(0, |index| index + 1);
        let early: Vec<_> = self.early_boxes.drain(early_start..).collect();
        // Lines that are not held but have blocks inside them paint
        // around those blocks, which the clip of the lines does not
        // cover.
        let around_blocks = !hold && !early.is_empty();
        let text_clip = self
            .geometry
            .text_page_clip(cascade, self.page_box, inside_fixed);
        if let Some(clip) = text_clip.as_ref().filter(|_| !around_blocks) {
            self.scene
                .scene
                .push_clip_layer(self.geometry.page_clip_transform, clip);
        }
        // A paragraph whose lines carry no visible text (only
        // atomic inlines and whitespace) is not traced as text,
        // as its whitespace-only text nodes have no fragments. The
        // text traced below the paragraph's blocks is not traced
        // again.
        let below = self.traced_below.remove(&node_id);
        let leaving = self.trace.is_some()
            && leaves_groups(&self.deferral_scopes)
            && self.paragraphs.can_leave(document, cascade, node_id);
        if let Some(t) = self.trace.as_deref_mut()
            && traced_text(
                document,
                cascade,
                node_id,
                |negative| !below || !negative,
                |owner| !leaving || !in_left_group(document, cascade, node_id, owner, None),
            )
        {
            t.push(crate::PaintTraceEvent::Text(node_id));
        }
        let saved_transform = self.scene.transform;
        self.scene.transform = paint_transform;
        if hold {
            let lines = self.record_lines(&frame);
            match lines {
                Some(mut lines) if boxes_in_groups || lines.has_groups() => {
                    for (_, id, content) in early {
                        if paint_rules::paints_before_paragraph_lines(
                            document, cascade, node_id, id,
                        ) {
                            lines.place_block(content);
                        } else {
                            lines.place_box(id, content);
                        }
                    }
                    self.held_lines.push(lines);
                }
                // Without groups the boxes paint in place: the early
                // ones below the lines, the others after them.
                lines => {
                    for (_, _, content) in early {
                        self.scene.scene.append_scene(content, Affine::IDENTITY);
                    }
                    if let Some(lines) = lines {
                        lines.finish(self.scene);
                    }
                }
            }
        } else if around_blocks {
            // The content positioned below the blocks paints first,
            // then the blocks, then the rest of the lines (CSS 2.1
            // Appendix E steps 3, 4 and 7).
            let (below, _, above, left) = self
                .record_lines(&frame)
                .map(|lines| lines.finish_leaving_groups(leaves_groups(&self.deferral_scopes)))
                .unwrap_or_default();
            leave_groups(
                document,
                &mut self.deferral_scopes,
                &mut self.left_groups,
                node_id,
                left,
                text_clip.map(|clip| (self.geometry.page_clip_transform, clip)),
            );
            for (content, blocks) in [(below, early), (above, Vec::new())] {
                if let Some(clip) = &text_clip {
                    self.scene
                        .scene
                        .push_clip_layer(self.geometry.page_clip_transform, clip);
                }
                self.scene.append_scene(content, Affine::IDENTITY);
                if text_clip.is_some() {
                    self.scene.pop_layer();
                }
                for (_, _, block) in blocks {
                    self.scene.scene.append_scene(block, Affine::IDENTITY);
                }
            }
        } else if let Some(lines) = self.record_lines(&frame) {
            if leaves_groups(&self.deferral_scopes) {
                let left = finish_lines_leaving_groups(self.scene, lines);
                leave_groups(
                    document,
                    &mut self.deferral_scopes,
                    &mut self.left_groups,
                    node_id,
                    left,
                    text_clip.map(|clip| (self.geometry.page_clip_transform, clip)),
                );
            } else {
                lines.finish(self.scene);
            }
        }
        self.scene.transform = saved_transform;
        if text_clip.is_some() && !around_blocks {
            self.scene.pop_layer();
        }
    }

    /// Lay out and paint the lines of the paragraph of `frame`, keeping them
    /// from being composited yet.
    fn record_lines(&mut self, frame: &LinesFrame) -> Option<HeldLines<'w>> {
        crate::ifc_text::hold_ifc_lines(
            self.image_cache,
            self.document,
            self.cascade,
            frame.node_id,
            IfcPosition {
                x: frame.x,
                y: frame.y,
                shift_y: frame.shift_y,
            },
            &frame.decorations,
            frame.fragmentainer,
            self.custom_highlights,
            self.pixel_source,
            self.warnings,
            self.page_box,
            self.geometry
                .page_lines
                .filter(|_| !frame.inside_fixed)
                .map(|lines| lines.translated(frame.translation_y)),
            frame.paint_transform,
            frame.fixed_viewport,
        )
    }
}
