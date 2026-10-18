//! The visit of an element: where its box lies on the page, what it paints
//! of itself, and the frames that paint its children.

use super::child_frames::{ChildFrames, ChildOrigin, ChildPlan, GeneratedOffsets};
use super::element_box::{BoxInputs, ElementBox};
use super::frames::{DeferralLayer, LayerClose, PaintFrame, VisitFrame};
use super::paint_walk::PaintWalk;
use crate::text;
use anyrender::PaintScene;
use raikiri_dom::Node;
use raikiri_style::property::{ColumnCountValue, DisplayValue};
use raikiri_traits::NodeId;

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Paint the element `node` of `visit`, laid out as `layout`, and push
    /// the frames that paint its children.
    pub(super) fn visit_element(
        &mut self,
        visit: &VisitFrame,
        node: &'w Node,
        layout: taffy::Layout,
    ) {
        let VisitFrame {
            node_id,
            parent_abs_x,
            parent_abs_y,
            parent_font_size,
            shift_y,
            transform_x,
            transform_y,
            paint_offset_y,
            fragment_clip_height,
            column_clip_height,
            fragment_id,
            fragment_origin_x,
            fragment_origin_y,
            fragmentainer,
            fragment_rect: _,
            fragment_clip,
            is_fragment_visit,
            inside_fixed,
            inside_fixed_containing_block,
            ref decorations,
        } = *visit;
        let (document, cascade) = (self.document, self.cascade);
        // An atomic inline an ellipsis removed from its line is not
        // painted (CSS Overflow 3 §5.1).
        if node.is_display_none() || node.is_hidden_by_text_overflow() {
            return;
        }
        let cv = &cascade.computed[node_id];
        let Some(element) = self.element_box(
            BoxInputs {
                node_id,
                parent_abs_x,
                parent_abs_y,
                parent_font_size,
                shift_y,
                transform_x,
                transform_y,
                fragment_clip_height,
                fragment_id,
                fragment_clip,
                is_fragment_visit,
                inside_fixed,
                inside_fixed_containing_block,
            },
            node,
            layout,
        ) else {
            return;
        };
        let ElementBox {
            abs_x,
            abs_y,
            generates_box,
            child_shift_y,
            pos_dx,
            pos_dy,
            fixed_dx,
            fixed_dy,
            child_transform_x,
            child_transform_y,
            placed_in_column,
            fixed_in_viewport,
            establishes_fixed_containing_block,
            multicol_clip_pushed,
            fragmentainer_clip_pushed,
            paints_on_page,
            ..
        } = element;
        self.open_opacity_group(&element);
        let has_column_rules = document.column_rules(cascade, node_id).next().is_some();
        let mut before_advance = if paints_on_page
            && !raikiri_dom::paint_rules::hides_empty_table_cell(document, cascade, node_id)
        {
            self.paint_box_decorations(&element);
            self.paint_replaced_content(&element);
            self.paint_column_rules(&element, has_column_rules);
            self.paint_list_marker(&element);
            self.paint_generated_before(&element, has_column_rules)
        } else {
            0.0
        };
        // Paint positioned siblings in stacking order while keeping
        // source order for equal stack levels. Flex/grid containers use
        // order-modified document order within each stacking bucket,
        // as required by CSS Display's `order` property behavior.
        // This is intentionally local to the current parent; full
        // nested stacking-context isolation remains outside this
        // minimal painter.
        let mut children =
            if node.is_inline_svg_root() || document.anonymous_table_contents_paint_only(node_id) {
                Vec::new()
            } else if node.is_ifc_root() {
                // The paragraph's own text and inline elements are drawn
                // from the lines; only the boxes laid out beside them are
                // visited like ordinary children.
                node.ifc_boxes()
            } else if let Some(children) = document.anonymous_table_paint_sequence(node_id) {
                children
            } else {
                node.children.clone()
            };
        raikiri_dom::paint_rules::sort_paint_children(&mut children, cv.display, cascade);
        let GeneratedOffsets {
            inline: inline_offsets,
            vertical: vertical_generated_offsets,
        } = self.generated_offsets(&element, &children);
        let overflow_clip_pushed = self.open_clips(&element);
        if let Some(advance) = self.paint_generated_overlay(&element, has_column_rules) {
            before_advance = advance;
        }
        let after_pushed = self.push_after(&element, before_advance);
        let paint_transform = self.scene.transform;
        let child_font_size = cv.font_size.px();
        // `text-decoration-line` is non-inherited at the computed-value
        // layer, but its originating line is propagated to descendants
        // by CSS Text Decoration. Keep that paint-only context separate
        // from `CascadeResult`'s inheritance result.
        let child_decorations = text::decorations_for_element(
            decorations,
            NodeId::new(node_id as u64),
            cv,
            child_shift_y,
        );

        // Reverse push makes the lowest stack level paint first.
        // Relative insets are already present in `layout.location`
        // except for the manual placement paths identified above.
        let child_parent_x = abs_x + pos_dx + fixed_dx;
        let child_parent_y = abs_y + pos_dy + fixed_dy;
        let child_paint_offset_y = paint_offset_y + pos_dy + fixed_dy;
        // The descendants of a box that is placed in a column are
        // placed from the origin of its fragment, which stands where
        // layout left it. The offset that paint adds to the box moves
        // them as well, as it does the descendants that follow the
        // parent chain.
        let (child_fragment_origin_x, child_fragment_origin_y) = if placed_in_column {
            (fragment_origin_x + pos_dx, fragment_origin_y + pos_dy)
        } else {
            (fragment_origin_x, fragment_origin_y)
        };
        // A paragraph inside a fixed box, or a fixed box that is a
        // paragraph itself, repeats on every page and is not clipped
        // like flowed text, as the text branch below does.
        let ifc_inside_fixed = inside_fixed || fixed_in_viewport;
        let origin = ChildOrigin {
            parent_x: child_parent_x,
            parent_y: child_parent_y,
            font_size: child_font_size,
            shift_y: child_shift_y,
            transform_x: child_transform_x,
            transform_y: child_transform_y,
            paint_offset_y: child_paint_offset_y,
            inside_fixed: ifc_inside_fixed,
            inside_fixed_containing_block: inside_fixed_containing_block
                || establishes_fixed_containing_block,
            fragment_id,
            fragment_origin_x: child_fragment_origin_x,
            fragment_origin_y: child_fragment_origin_y,
            fragmentainer,
            decorations: &child_decorations,
        };
        let lines_frame = self.paragraph_lines(&element, &origin, &children, paint_transform);
        let hold_lines = lines_frame.as_ref().is_some_and(|lines| lines.hold);
        let lines_frame = lines_frame.map(PaintFrame::Lines);
        let own_multicol_clip_height = match cv.column_count {
            ColumnCountValue::Count(count)
                if count > 1 && node.is_multicol_container() && layout.size.height > 0.0 =>
            {
                Some(layout.size.height)
            }
            _ => None,
        };
        // A `display: contents` element has no box, so the columns of
        // the container it sits in hold its children.
        let own_multicol_clip_height = own_multicol_clip_height
            .or((cv.display == DisplayValue::Contents)
                .then_some(column_clip_height)
                .flatten());
        // A plain block in a block container leaves its inline
        // content to the end of the layer (CSS 2.1 Appendix E): its
        // lines and the boxes laid out beside them paint after the
        // backgrounds of the blocks that follow it.
        let defers_here = !multicol_clip_pushed
            && !fragmentainer_clip_pushed
            && fragmentainer.is_none()
            && !is_fragment_visit
            && !fixed_in_viewport;
        let defers = defers_here
            && overflow_clip_pushed.is_none()
            && raikiri_dom::paint_rules::defers_inline_content(document, cascade, node_id);
        // A plain block that clips collects its inline content in a
        // layer of its own, which it hands to the enclosing layer
        // inside its clip.
        let merge = overflow_clip_pushed
            .filter(|_| {
                defers_here
                    && self
                        .deferral_scopes
                        .last()
                        .is_some_and(|layer| layer.allowed)
                    && raikiri_dom::paint_rules::merges_clipped_inline_content(
                        document, cascade, node_id,
                    )
            })
            .map(|(clip, transform)| LayerClose::Merge {
                node_id,
                clip,
                transform,
            });
        // With content positioned below the blocks, the lines place
        // the blocks between that content and the rest.
        let negative_content = node.is_ifc_root()
            && raikiri_dom::paint_rules::holds_negative_stack_level(document, cascade, node_id);
        let ChildFrames {
            children: child_frames,
            below_lines: below_lines_frames,
            blocks: mut block_frames,
        } = self.child_frames(
            &element,
            &origin,
            children,
            &GeneratedOffsets {
                inline: inline_offsets,
                vertical: vertical_generated_offsets,
            },
            &ChildPlan {
                fragment_clip_height,
                multicol_clip_height: own_multicol_clip_height,
                before_advance,
                defers,
                hold_lines,
                negative_content,
            },
        );
        let finish_lines = hold_lines.then_some(PaintFrame::FinishLines { root: node_id });
        let stack = &mut self.stack;
        let deferral_scopes = &mut self.deferral_scopes;
        // A `display: contents` element has no box, so its children
        // share the enclosing layer.
        if !defers && generates_box {
            let allowed = deferral_scopes.last().is_some_and(|layer| layer.allowed)
                && !node.is_multicol_container();
            let close = merge.unwrap_or(
                if raikiri_dom::paint_rules::hoists_positioned_content(document, cascade, node_id) {
                    LayerClose::Hoist
                } else {
                    LayerClose::Paint
                },
            );
            let mut layer = DeferralLayer::new(allowed, close);
            layer.after = after_pushed;
            deferral_scopes.push(layer);
            stack.push(PaintFrame::EndDeferral);
        }
        if (defers || merge.is_some()) && node.is_ifc_root() {
            if let Some(layer) = deferral_scopes.last_mut() {
                let pending = &mut layer.pending;
                pending.extend(below_lines_frames.into_iter().rev());
                if negative_content && !block_frames.is_empty() {
                    pending.push(PaintFrame::TraceBelowBlocks { root: node_id });
                    pending.extend(std::mem::take(&mut block_frames).into_iter().rev());
                }
                pending.extend(lines_frame);
                pending.extend(child_frames.into_iter().rev());
                pending.extend(finish_lines);
                // A standalone `::after` still follows the content
                // it is generated after.
                if defers && after_pushed {
                    pending.extend(stack.pop());
                }
            }
        } else {
            stack.extend(finish_lines);
            stack.extend(child_frames);
            stack.extend(lines_frame);
            if negative_content && !block_frames.is_empty() {
                stack.extend(std::mem::take(&mut block_frames));
                stack.push(PaintFrame::TraceBelowBlocks { root: node_id });
            }
            stack.extend(below_lines_frames);
        }
        stack.extend(block_frames);
    }
}
