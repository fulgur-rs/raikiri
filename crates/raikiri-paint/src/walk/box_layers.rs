//! The groups, clips and layers that enclose the paint of an element and its
//! subtree.

use super::child_frames::{ChildFrames, ChildPlan};
use super::element_box::ElementBox;
use super::frames::{DeferralLayer, LayerClose, PaintFrame};
use super::paint_walk::{ClipScope, PaintWalk};
use super::{paint_generated_pseudo, push_overflow_clip};
use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use peniko::Mix;
use raikiri_traits::{PaintInsets, PaintRect};

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Open the opacity group of `element` when it is translucent, and
    /// push the frame that closes it after the complete subtree.
    pub(super) fn open_opacity_group(&mut self, element: &ElementBox<'w>) {
        let (cv, node_id) = (element.cv, element.node_id);
        let page_box = self.page_box;
        let scene = &mut *self.scene;
        let trace = &mut self.trace;
        let stack = &mut self.stack;
        // CSS Color 4 §3.3: opacity applies to the complete
        // element group, including its background, border, text,
        // generated content, and descendants. Keep this layer open
        // until the deferred `PaintAfter` and overflow clip frames
        // have finished so overlapping descendants are composited as
        // one group instead of being alpha-blended individually.
        let has_opacity_layer =
            raikiri_dom::paint_rules::element_opacity_layer(self.document, node_id, cv).is_some();
        if has_opacity_layer {
            let opacity_clip = Rect::new(
                0.0,
                0.0,
                page_box.width.max(0.0) as f64,
                page_box.height.max(0.0) as f64,
            );
            scene.scene.push_layer(
                Mix::Normal,
                cv.opacity,
                Affine::IDENTITY,
                &opacity_clip,
                None,
                None,
            );
            // This frame is pushed first so it closes after the
            // optional overflow clip and generated `::after` paint.
            stack.push(PaintFrame::PopOpacity);
            if let Some(t) = trace.as_deref_mut() {
                t.push(crate::PaintTraceEvent::PushOpacity(node_id, cv.opacity));
            }
        }
    }

    /// Push the frames that close the clips of `element` after its subtree,
    /// and open its overflow clip. Return that clip and the transform it was
    /// pushed in.
    pub(super) fn open_clips(
        &mut self,
        element: &ElementBox<'w>,
    ) -> Option<(raikiri_traits::PaintClip, Affine)> {
        let ElementBox {
            node_id,
            layout,
            paint_x,
            paint_y,
            paint_height,
            multicol_clip_pushed,
            fragmentainer_clip_pushed,
            ..
        } = *element;
        let (document, cascade, page_box) = (self.document, self.cascade, self.page_box);
        // The fragmentainer clip encloses the element and its children;
        // close it after any element-local overflow clip.
        if multicol_clip_pushed {
            self.stack.push(PaintFrame::PopClip);
            self.clip_scopes.push(ClipScope::Eager);
        }
        if fragmentainer_clip_pushed {
            self.stack.push(PaintFrame::PopClip);
            self.clip_scopes.push(ClipScope::Eager);
        }
        // CSS Overflow 3 §3.1: non-visible overflow clips descendants to
        // the padding box. Min edges are floored outward so a fractional
        // padding-box origin does not antialias-cut pixel-snapped descendant
        // backgrounds (which round to integers); this mirrors the multicol
        // clip origin flooring. The clip is pushed only after painting the
        // element itself, then popped after its complete subtree via the
        // explicit stack frame.
        let mut overflow_clip_pushed = None;
        if let Some(resolved) = raikiri_dom::paint_rules::overflow_clip(
            document,
            cascade,
            node_id,
            PaintRect::new(paint_x, paint_y, layout.size.width, paint_height),
            PaintInsets::new(
                layout.border.top,
                layout.border.right,
                layout.border.bottom,
                layout.border.left,
            ),
        ) {
            self.begin_clip_scope(!element.paints_on_page);
            let transform = self.scene.transform;
            let clip = push_overflow_clip(self.scene, resolved, &page_box);
            self.stack.push(PaintFrame::PopClip);
            overflow_clip_pushed = Some((resolved, transform));
            if let Some(t) = self.trace.as_deref_mut() {
                t.push(crate::PaintTraceEvent::PushOverflowClip(
                    node_id,
                    [clip.x0, clip.y0, clip.x1, clip.y1],
                ));
            }
        }
        overflow_clip_pushed
    }

    /// Paint the `::before` content of `element` again, inside its overflow
    /// clip, when it has column rules, and return its advance.
    pub(super) fn paint_generated_overlay(
        &mut self,
        element: &ElementBox<'w>,
        has_column_rules: bool,
    ) -> Option<f32> {
        let ElementBox {
            node_id,
            layout,
            paint_height,
            paints_on_page,
            paints_as_absolute_continuation,
            generated_x,
            generated_y,
            ..
        } = *element;
        let (document, cascade, counter_snapshots) =
            (self.document, self.cascade, self.counter_snapshots);
        // A column rule is below all content, including a standalone
        // generated overlay that the inline engine did not lay out.
        if has_column_rules
            && paints_on_page
            && !paints_as_absolute_continuation
            && !raikiri_dom::paint_rules::hides_empty_table_cell(document, cascade, node_id)
        {
            Some(paint_generated_pseudo(
                &mut *self.scene,
                document,
                cascade,
                node_id,
                raikiri_style::PseudoElem::Before,
                generated_x,
                generated_y,
                layout.size.width,
                paint_height,
                counter_snapshots,
            ))
        } else {
            None
        }
    }

    /// Push the frame that paints the `::after` content of `element` after
    /// its children, and say whether it was pushed.
    pub(super) fn push_after(&mut self, element: &ElementBox<'w>, before_advance: f32) -> bool {
        let ElementBox {
            node_id,
            layout,
            paint_height,
            paints_on_page,
            paints_as_absolute_continuation,
            generated_x,
            generated_y,
            generated_content_advance,
            ..
        } = *element;
        // Push this after the clip-pop frame and before children.
        // Children therefore paint first, then `::after`, then the
        // clip closes.  A full IFC would place the after run after
        // laid-out inline descendants; this minimal path uses the
        // generated-before advance as its only inline position.
        let after_pushed = paints_on_page && !paints_as_absolute_continuation;
        if after_pushed {
            self.stack.push(PaintFrame::PaintAfter {
                node_id,
                x: generated_x,
                y: generated_y,
                width: layout.size.width,
                height: paint_height,
                before_advance,
                content_advance: generated_content_advance,
            });
        }
        after_pushed
    }

    /// Open the layer of `element` when it has one, and put its frames on
    /// the stack, or in the layer when its inline content waits for the end.
    pub(super) fn place_frames(
        &mut self,
        element: &ElementBox<'w>,
        plan: &ChildPlan,
        frames: ChildFrames<'w>,
        lines_frame: Option<PaintFrame<'w>>,
    ) {
        let ElementBox {
            node,
            node_id,
            generates_box,
            ..
        } = *element;
        let ChildPlan {
            defers,
            hold_lines,
            negative_content,
            merge,
            after_pushed,
            ..
        } = *plan;
        let (document, cascade) = (self.document, self.cascade);
        let ChildFrames {
            children: child_frames,
            below_lines: below_lines_frames,
            blocks: mut block_frames,
        } = frames;
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
