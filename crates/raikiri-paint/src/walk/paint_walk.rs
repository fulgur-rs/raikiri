//! The state that the frames of the paint stack change, and the frames that
//! change nothing else.

use super::frames::{
    DeferralLayer, LayerClose, PaintFrame, finish_lines_leaving_groups, leave_groups,
    leaves_groups, positioned_frame,
};
use super::{in_left_group, paint_generated_pseudo, push_overflow_clip, traced_text};
use crate::capture::CaptureScene;
use crate::transform::TransformScene;
use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use peniko::Mix;
use raikiri_dom::paint_rules;
use raikiri_dom::{CounterSnapshots, Document};
use raikiri_style::CascadeResult;
use raikiri_traits::{PageBox, PaintClip};
use std::collections::{HashMap, HashSet};

/// The walk of one page: what it paints into and the work that waits for
/// frames further down the stack.
pub(super) struct PaintWalk<'w, S> {
    pub(super) document: &'w Document,
    pub(super) cascade: &'w CascadeResult,
    pub(super) counter_snapshots: &'w CounterSnapshots,
    pub(super) page_box: PageBox,
    pub(super) scene: &'w mut TransformScene<'w, CaptureScene<'w, S>>,
    pub(super) trace: Option<&'w mut Vec<crate::PaintTraceEvent>>,
    /// Paragraphs whose groups wait for the boxes beside their lines,
    /// innermost last.
    pub(super) held_lines: Vec<crate::ifc_text::HeldLines<'w>>,
    /// Boxes recorded before the lines of their paragraph, which place them:
    /// the paragraph root, the box and its paint.
    pub(super) early_boxes: Vec<(usize, usize, anyrender::Scene)>,
    /// The paint of the inline groups waiting for the end of their layer (see
    /// [`PaintFrame::InlineGroup`]), in page space, by element.
    pub(super) left_groups: HashMap<usize, anyrender::Scene>,
    /// Paragraphs whose text below their blocks was traced before the blocks.
    pub(super) traced_below: HashSet<usize>,
    /// Each open layer collects the content that waits for its end. The
    /// outermost layer closes after the whole body.
    pub(super) deferral_scopes: Vec<DeferralLayer<'w>>,
    pub(super) stack: Vec<PaintFrame<'w>>,
}

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Carry out `frame` if it changes nothing but the state of the walk, and
    /// give it back if it has boxes or lines of its own to paint.
    pub(super) fn step(&mut self, frame: PaintFrame<'w>) -> Option<PaintFrame<'w>> {
        match frame {
            PaintFrame::RestoreTransform(transform) => self.scene.transform = transform,
            PaintFrame::EndDeferral => self.end_deferral(),
            PaintFrame::PushClip {
                node_id,
                clip,
                transform,
            } => self.push_clip(node_id, clip, transform),
            PaintFrame::BeginBox {
                root,
                node_id,
                early,
            } => self.begin_box(root, node_id, early),
            PaintFrame::EndBox {
                root,
                node_id,
                early,
            } => self.end_box(root, node_id, early),
            PaintFrame::TraceBelowBlocks { root } => self.trace_below_blocks(root),
            PaintFrame::FinishLines { root } => self.finish_lines(root),
            PaintFrame::InlineGroup { root, node_id } => self.inline_group(root, node_id),
            PaintFrame::PopClip => self.pop_clip(),
            PaintFrame::PushOpacity { node_id, alpha } => self.push_opacity(node_id, alpha),
            PaintFrame::PopOpacity => self.pop_opacity(),
            PaintFrame::PaintAfter {
                node_id,
                x,
                y,
                width,
                height,
                before_advance,
                content_advance,
            } => {
                let _ = paint_generated_pseudo(
                    self.scene,
                    self.document,
                    self.cascade,
                    node_id,
                    raikiri_style::PseudoElem::After,
                    x + before_advance + content_advance,
                    y,
                    (width - before_advance).max(0.0),
                    height,
                    self.counter_snapshots,
                );
            }
            frame => return Some(frame),
        }
        None
    }

    /// Close the innermost layer of deferred content: paint what it deferred,
    /// then drop it.
    fn end_deferral(&mut self) {
        let (document, cascade) = (self.document, self.cascade);
        // Keep the layer open while its content paints, so a box
        // among it that closes deferred content flushes into it.
        let replay = self
            .deferral_scopes
            .last_mut()
            .map(|layer| match layer.close {
                LayerClose::Merge { .. } => Vec::new(),
                _ if !layer.pending.is_empty() => std::mem::take(&mut layer.pending),
                LayerClose::Paint => {
                    layer.flushing = true;
                    let positioned = std::mem::take(&mut layer.positioned);
                    layer
                        .queue
                        .get_or_insert_with(|| {
                            paint_rules::PositionedQueue::new(
                                document,
                                cascade,
                                positioned,
                                positioned_frame,
                            )
                        })
                        .next(PaintFrame::PopClip)
                }
                LayerClose::Hoist => Vec::new(),
            })
            .unwrap_or_default();
        if !replay.is_empty() {
            self.stack.push(PaintFrame::EndDeferral);
            self.stack.extend(replay.into_iter().rev());
            return;
        }
        let closed = self.deferral_scopes.pop();
        if let (Some(layer), Some(outer)) = (closed, self.deferral_scopes.last_mut()) {
            match layer.close {
                LayerClose::Paint => {}
                LayerClose::Merge {
                    node_id,
                    clip,
                    transform,
                } => {
                    let mut pending = layer.pending;
                    if layer.after
                        && !pending.is_empty()
                        && matches!(self.stack.last(), Some(PaintFrame::PaintAfter { .. }))
                    {
                        pending.extend(self.stack.pop());
                    }
                    for (frames, into) in [
                        (pending, &mut outer.pending),
                        (layer.positioned, &mut outer.positioned),
                    ] {
                        if !frames.is_empty() {
                            into.push(PaintFrame::PushClip {
                                node_id,
                                clip,
                                transform,
                            });
                            into.extend(frames);
                            into.push(PaintFrame::PopClip);
                        }
                    }
                }
                // A box the enclosing layer paints at its end hands on
                // its boxes to wait with the boxes still to paint.
                LayerClose::Hoist if outer.queue.is_some() => {
                    if let Some(queue) = &mut outer.queue {
                        queue.hand_on(document, cascade, layer.positioned, positioned_frame);
                    }
                }
                LayerClose::Hoist => outer.positioned.extend(layer.positioned),
            }
        }
    }

    /// Reopen the overflow clip of a block, in the transform the block was
    /// painted in.
    fn push_clip(&mut self, node_id: usize, clip: PaintClip, transform: Affine) {
        let saved_transform = self.scene.transform;
        self.scene.transform = transform;
        let rect = push_overflow_clip(self.scene, clip, &self.page_box);
        self.scene.transform = saved_transform;
        if let Some(t) = self.trace.as_deref_mut() {
            t.push(crate::PaintTraceEvent::PushOverflowClip(
                node_id,
                [rect.x0, rect.y0, rect.x1, rect.y1],
            ));
        }
    }

    /// Start recording the paint of box `node_id`, laid out beside the lines
    /// of paragraph `root`.
    fn begin_box(&mut self, root: usize, node_id: usize, early: bool) {
        let (document, cascade) = (self.document, self.cascade);
        // A box that closes deferred inline content paints it before
        // the recording starts, as its visit would.
        if self
            .deferral_scopes
            .last()
            .is_some_and(|layer| !layer.pending.is_empty())
            && paint_rules::closes_deferred_inline_content(document, cascade, node_id)
        {
            let pending = self
                .deferral_scopes
                .last_mut()
                .map(|layer| std::mem::take(&mut layer.pending))
                .unwrap_or_default();
            self.stack.push(PaintFrame::BeginBox {
                root,
                node_id,
                early,
            });
            self.stack.extend(pending.into_iter().rev());
            return;
        }
        // The trace records the box inside the opacities of the
        // inline elements around it, as it does for boxes beside
        // lines that are not held.
        if let Some(t) = self.trace.as_deref_mut() {
            t.extend(
                crate::ifc_text::enclosing_inline_opacities(
                    crate::ifc_text::Paragraph {
                        document,
                        cascade,
                        root_id: root,
                    },
                    node_id,
                )
                .into_iter()
                .map(|(id, alpha)| crate::PaintTraceEvent::PushOpacity(id, alpha)),
            );
        }
        if early
            || self
                .held_lines
                .last()
                .is_some_and(|lines| lines.root_id() == root)
        {
            self.scene.scene.begin_capture();
        }
    }

    /// Place the recorded paint of box `node_id` in the held lines of `root`.
    fn end_box(&mut self, root: usize, node_id: usize, early: bool) {
        if let Some(t) = self.trace.as_deref_mut() {
            let opacities = crate::ifc_text::enclosing_inline_opacities(
                crate::ifc_text::Paragraph {
                    document: self.document,
                    cascade: self.cascade,
                    root_id: root,
                },
                node_id,
            );
            t.extend(opacities.iter().map(|_| crate::PaintTraceEvent::PopOpacity));
        }
        if early {
            self.early_boxes
                .push((root, node_id, self.scene.scene.end_capture()));
        } else if let Some(lines) = self
            .held_lines
            .last_mut()
            .filter(|lines| lines.root_id() == root)
        {
            lines.place_box(node_id, self.scene.scene.end_capture());
        }
    }

    /// Trace the text of paragraph `root` positioned below its blocks, which
    /// its held lines paint below the blocks placed in them.
    fn trace_below_blocks(&mut self, root: usize) {
        if let Some(t) = self.trace.as_deref_mut() {
            self.traced_below.insert(root);
            if traced_text(
                self.document,
                self.cascade,
                root,
                |negative| negative,
                |_| true,
            ) {
                t.push(crate::PaintTraceEvent::Text(root));
            }
        }
    }

    /// Composite the held lines of `root` and the boxes placed in them.
    fn finish_lines(&mut self, root: usize) {
        if self
            .held_lines
            .last()
            .is_some_and(|lines| lines.root_id() == root)
            && let Some(lines) = self.held_lines.pop()
        {
            let saved_transform = self.scene.transform;
            self.scene.transform = lines.paint_transform();
            if leaves_groups(&self.deferral_scopes) {
                let left = finish_lines_leaving_groups(self.scene, lines);
                leave_groups(
                    self.document,
                    &mut self.deferral_scopes,
                    &mut self.left_groups,
                    root,
                    left,
                    None,
                );
            } else {
                lines.finish(self.scene);
            }
            self.scene.transform = saved_transform;
        }
    }

    /// Paint the group of positioned inline element `node_id`, which its
    /// paragraph's lines left to the end of the layer.
    fn inline_group(&mut self, root: usize, node_id: usize) {
        let (document, cascade) = (self.document, self.cascade);
        // The text of one paragraph painted at once is one step, even
        // when it spans several groups.
        if let Some(t) = self.trace.as_deref_mut()
            && t.last() != Some(&crate::PaintTraceEvent::Text(root))
            && traced_text(
                document,
                cascade,
                root,
                |_| true,
                |owner| in_left_group(document, cascade, root, owner, Some(node_id)),
            )
        {
            t.push(crate::PaintTraceEvent::Text(root));
        }
        let content = self.left_groups.remove(&node_id).unwrap_or_default();
        self.scene.scene.append_scene(content, Affine::IDENTITY);
    }

    fn pop_clip(&mut self) {
        if let Some(t) = self.trace.as_deref_mut() {
            t.push(crate::PaintTraceEvent::PopClip);
        }
        self.scene.pop_layer();
    }

    /// Open the opacity group of a translucent inline element around a box
    /// laid out beside its paragraph's lines.
    fn push_opacity(&mut self, node_id: usize, alpha: f32) {
        if let Some(t) = self.trace.as_deref_mut() {
            t.push(crate::PaintTraceEvent::PushOpacity(node_id, alpha));
        }
        self.scene.scene.push_layer(
            Mix::Normal,
            alpha,
            Affine::IDENTITY,
            &Rect::new(
                0.0,
                0.0,
                f64::from(self.page_box.width.max(0.0)),
                f64::from(self.page_box.height.max(0.0)),
            ),
            None,
            None,
        );
    }

    fn pop_opacity(&mut self) {
        if let Some(t) = self.trace.as_deref_mut() {
            t.push(crate::PaintTraceEvent::PopOpacity);
        }
        self.scene.pop_layer();
    }
}
