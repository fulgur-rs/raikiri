//! The walk of one page's nodes in paint order. It keeps an explicit stack of
//! frames, like the painter's, so a deep DOM cannot overflow the call stack.

use super::{
    ClipKind, Frame, Layer, LayerClose, LinePart, PageItems, PaintEvent, ParagraphFrames,
    TextLines, list_paragraph, own_box_events, positioned_frame, pseudo_box, push_paragraph,
};
use crate::page_projection::paint_transform::{self, Motion, MotionCache};
use crate::page_projection::records::{PageFragmentItem, PageFragmentKind};
use crate::{Document, paint_rules};
use raikiri_style::{CascadeResult, PseudoElem};
use raikiri_traits::{NodeId, NodeKind, PaintClip, PaintRect, PaintTransform};
use std::collections::HashSet;

/// The state of the walk of one page.
pub(super) struct PaintWalk<'a, 'l> {
    document: &'a Document,
    cascade: &'a CascadeResult,
    items: &'l PageItems<'a>,
    lines: Option<&'l TextLines>,
    page_index: u32,
    /// The nodes that list something on the page, when the walk skips the
    /// others.
    entered: Option<HashSet<usize>>,
    events: Vec<PaintEvent<'a>>,
    /// Paragraph content that waits for the stacking contexts painted below
    /// it, by `Frame::HeldContent` index.
    held: Vec<Vec<PaintEvent<'a>>>,
    stack: Vec<Frame>,
    /// Each open layer collects the content that waits for its end.
    layers: Vec<Layer>,
    /// Whether each paragraph's content can leave its lines, found once for
    /// all its groups.
    paragraphs: paint_rules::LeavingParagraphs,
    motion: MotionCache,
    /// The transforms open around the current event, composed.
    spaces: Vec<PaintTransform>,
}

impl<'a, 'l> PaintWalk<'a, 'l> {
    /// A walk of the page of `items` that starts at the document element,
    /// `root`.
    pub(super) fn new(
        document: &'a Document,
        cascade: &'a CascadeResult,
        items: &'l PageItems<'a>,
        lines: Option<&'l TextLines>,
        page_index: u32,
        entered: Option<HashSet<usize>>,
        root: usize,
    ) -> Self {
        Self {
            document,
            cascade,
            items,
            lines,
            page_index,
            entered,
            events: Vec::new(),
            held: Vec::new(),
            stack: vec![Frame::EndDeferral, Frame::Visit(root)],
            layers: vec![Layer::new(true, LayerClose::Paint)],
            paragraphs: paint_rules::LeavingParagraphs::default(),
            motion: MotionCache::default(),
            spaces: vec![PaintTransform::IDENTITY],
        }
    }

    /// The events of the page, before the clips cut them.
    pub(super) fn run(mut self) -> Vec<PaintEvent<'a>> {
        while let Some(frame) = self.stack.pop() {
            self.step(frame);
        }
        self.events
    }

    fn step(&mut self, frame: Frame) {
        if self.defers_box(frame)
            || self.defers_inline_group(frame)
            || self.closes_deferred_content(frame)
        {
            return;
        }
        match frame {
            Frame::EndDeferral => self.end_deferral(),
            Frame::DeferredParagraph(key) => {
                if let Some(layer) = self.layers.last_mut() {
                    layer.pending.push(Frame::Paragraph(key));
                }
            }
            Frame::PushClip(owner) => {
                if let Some(entry) = self.items.overflow_clips.get(&NodeId::new(owner as u64)) {
                    self.events
                        .push(PaintEvent::PushClip(entry.clip, ClipKind::Overflow));
                }
            }
            Frame::Content(node_id) => self.content(node_id),
            Frame::Paragraph(key) => self.paragraph(key),
            Frame::FirstLine(root) => self.first_line(root),
            Frame::InlineGroup(root, group, part) => self.inline_group(root, group, part),
            Frame::HeldContent(index) => self.events.append(&mut self.held[index]),
            Frame::PopClip => self.events.push(PaintEvent::PopClip),
            Frame::PopOpacity => self.events.push(PaintEvent::PopOpacity),
            Frame::PopTransform => {
                self.events.push(PaintEvent::PopTransform);
                self.spaces.pop();
            }
            Frame::Visit(node_id) => self.visit(node_id),
        }
    }

    /// The transform around the current event.
    fn space(&self) -> PaintTransform {
        *self.spaces.last().unwrap_or(&PaintTransform::IDENTITY)
    }

    /// Open `transform` around the events listed until its frame pops.
    fn push_transform(&mut self, transform: PaintTransform) {
        let space = self.space();
        self.events.push(PaintEvent::PushTransform(transform));
        self.stack.push(Frame::PopTransform);
        self.spaces.push(space * transform);
    }

    /// Open an opacity group around the events listed until its frame pops.
    fn push_opacity(&mut self, alpha: f32) {
        self.events.push(PaintEvent::PushOpacity(alpha));
        self.stack.push(Frame::PopOpacity);
    }

    /// List the inline content of a paragraph between the frames around it.
    fn list_content(&mut self, content: Vec<PaintEvent<'a>>, own: usize, frames: ParagraphFrames) {
        list_paragraph(
            content,
            own,
            frames,
            &mut self.events,
            &mut self.held,
            &mut self.stack,
        );
    }

    /// Whether `frame` is a positioned box that waits for the end of its
    /// layer, after the backgrounds of the blocks that follow it. It is put
    /// aside then.
    fn defers_box(&mut self, frame: Frame) -> bool {
        if let Frame::Visit(node_id) = frame
            && let Some(layer) = self.layers.last_mut()
            && layer.allowed
            && !layer.flushing
            && paint_rules::defers_to_positioned_layer(
                self.document,
                self.cascade,
                node_id,
                &self.paragraphs,
            )
        {
            if paint_rules::leaves_paragraph_lines(
                self.document,
                self.cascade,
                node_id,
                &self.paragraphs,
            ) {
                paint_rules::insert_positioned_frame(
                    self.document,
                    &mut layer.positioned,
                    frame,
                    positioned_frame,
                );
            } else {
                layer.positioned.push(frame);
            }
            return true;
        }
        false
    }

    /// Whether `frame` is the group of a positioned inline element that forms
    /// a stacking context in the paragraph's stacking context. It waits for
    /// the end of the layer too.
    fn defers_inline_group(&mut self, frame: Frame) -> bool {
        if let Frame::InlineGroup(root, group, _) = frame
            && let Some(layer) = self.layers.last_mut()
            && layer.allowed
            && !layer.flushing
            && paint_rules::leaves_paragraph_content(
                self.document,
                self.cascade,
                root,
                group,
                self.paragraphs.can_leave(self.document, self.cascade, root),
            )
        {
            paint_rules::insert_positioned_frame(
                self.document,
                &mut layer.positioned,
                frame,
                positioned_frame,
            );
            return true;
        }
        false
    }

    /// Whether `frame` is a box that does not defer, such as a flex
    /// container, and so paints after the inline content deferred before it.
    /// That content is listed first, and the box is visited again after it.
    fn closes_deferred_content(&mut self, frame: Frame) -> bool {
        if let Frame::Visit(node_id) = frame
            && self
                .layers
                .last()
                .is_some_and(|layer| !layer.pending.is_empty())
            && paint_rules::closes_deferred_inline_content(self.document, self.cascade, node_id)
        {
            let pending = self
                .layers
                .last_mut()
                .map(|layer| std::mem::take(&mut layer.pending))
                .unwrap_or_default();
            self.stack.push(frame);
            self.stack.extend(pending.into_iter().rev());
            return true;
        }
        false
    }

    /// Close the layer on top: list what its blocks deferred, then drop it.
    fn end_deferral(&mut self) {
        let replay = self.layer_replay();
        if !replay.is_empty() {
            self.stack.push(Frame::EndDeferral);
            self.stack.extend(replay.into_iter().rev());
            return;
        }
        self.close_layer();
    }

    /// The frames the layer on top still has to list before it can close.
    fn layer_replay(&mut self) -> Vec<Frame> {
        let (document, cascade) = (self.document, self.cascade);
        self.layers
            .last_mut()
            .map(|layer| match layer.close {
                LayerClose::Merge(_) => Vec::new(),
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
                        .next(Frame::PopClip)
                }
                LayerClose::Hoist => Vec::new(),
            })
            .unwrap_or_default()
    }

    /// Drop the layer on top, handing what it collected to the layer around
    /// it as its way of closing says.
    fn close_layer(&mut self) {
        let closed = self.layers.pop();
        let (Some(layer), Some(outer)) = (closed, self.layers.last_mut()) else {
            return;
        };
        match layer.close {
            LayerClose::Paint => {}
            LayerClose::Merge(owner) => {
                for (frames, into) in [
                    (layer.pending, &mut outer.pending),
                    (layer.positioned, &mut outer.positioned),
                ] {
                    if !frames.is_empty() {
                        into.push(Frame::PushClip(owner));
                        into.extend(frames);
                        into.push(Frame::PopClip);
                    }
                }
            }
            // A box the enclosing layer lists at its end hands on its boxes
            // to wait with the boxes still to list.
            LayerClose::Hoist => match &mut outer.queue {
                Some(queue) => {
                    queue.hand_on(
                        self.document,
                        self.cascade,
                        layer.positioned,
                        positioned_frame,
                    );
                }
                None => outer.positioned.extend(layer.positioned),
            },
        }
    }

    /// The inline content of paragraph root `node_id`.
    fn content(&mut self, node_id: usize) {
        let (document, cascade, items, lines) =
            (self.document, self.cascade, self.items, self.lines);
        // The supplied runs tell the first line apart; text fragments cover
        // all the lines of a text node.
        let first_line =
            lines.is_some() && paint_rules::first_line_opacity_layer(document, node_id).is_some();
        let part = if first_line {
            LinePart::Rest
        } else {
            LinePart::All
        };
        let mut content = Vec::new();
        // The marker is inside the first line.
        if !first_line {
            content.extend(self.inline_marker_image(node_id));
        }
        let groups = push_paragraph(document, items, node_id, None, lines, part, &mut content);
        let groups = document.groups_with_content(items, node_id, groups, lines, part);
        let boxes = document.ifc_boxes_in_group(cascade, node_id, None, part);
        let mut frames =
            document.paragraph_frames(cascade, node_id, boxes, groups, part, first_line);
        // The blocks inside the paragraph paint above the content positioned
        // below them and below the rest of it.
        if paint_rules::holds_negative_stack_level(document, cascade, node_id) {
            frames.blocks = document
                .paragraph_blocks(cascade, node_id)
                .into_iter()
                .map(Frame::Visit)
                .collect();
        }
        self.list_content(content, 0, frames);
    }

    /// The raster marker of list item `root` when the page paints it with
    /// the item's lines, inside the first line.
    fn inline_marker_image(&self, root: usize) -> Option<PaintEvent<'a>> {
        self.document
            .has_inline_marker_image(self.cascade, self.page_index, root)
            .then_some(PaintEvent::MarkerImage(NodeId::new(root as u64)))
    }

    /// The paragraph of anonymous cell `key`.
    fn paragraph(&mut self, key: usize) {
        let (document, cascade, items, lines) =
            (self.document, self.cascade, self.items, self.lines);
        let mut content = Vec::new();
        let groups = push_paragraph(
            document,
            items,
            key,
            None,
            lines,
            LinePart::All,
            &mut content,
        );
        let frames =
            document.paragraph_frames(cascade, key, Vec::new(), groups, LinePart::All, false);
        self.list_content(content, 0, frames);
    }

    /// The group of the translucent `::first-line` of paragraph `root`. It
    /// composites the first line's content: its text, inline boxes,
    /// generated content and atomic inlines, and the first-line parts of the
    /// groups on it. The other boxes laid out beside the lines stay with the
    /// rest.
    fn first_line(&mut self, root: usize) {
        let (document, cascade, items, lines) =
            (self.document, self.cascade, self.items, self.lines);
        let Some(alpha) = paint_rules::first_line_opacity_layer(document, root) else {
            return; // cov:ignore: the frame is listed only for a translucent first line
        };
        let mut content = Vec::from_iter(self.inline_marker_image(root));
        let part = LinePart::First;
        let nested = push_paragraph(document, items, root, None, lines, part, &mut content);
        let nested = document.groups_with_content(items, root, nested, lines, part);
        let boxes = document.ifc_boxes_in_group(cascade, root, None, part);
        // A first line on another page lists nothing here.
        if content.is_empty()
            && nested.is_empty()
            && !boxes.iter().any(|id| items.by_node.contains_key(id))
        {
            return;
        }
        self.push_opacity(alpha);
        let frames = document.paragraph_frames(cascade, root, boxes, nested, part, false);
        self.list_content(content, 0, frames);
    }

    /// The opacity group `group` of paragraph `root`. It composites after
    /// the paragraph's own content, like other `z-index: auto` stacking
    /// contexts: its inline boxes and text, then its atomic inlines and
    /// floats, then the groups nested in it.
    fn inline_group(&mut self, root: usize, group: usize, part: LinePart) {
        let (document, cascade, items, lines) =
            (self.document, self.cascade, self.items, self.lines);
        let alpha = self.group_opacity(root, group);
        // A `::before` or `::after` with a transform turns its group about
        // its own box (CSS Transforms 1 §6). A singular one paints nothing of
        // it (CSS Transforms 1 §5). Without a box on the page, such as an
        // empty one, its content is listed where layout put it, as its runs
        // are.
        let turned = paint_rules::pseudo_transform(cascade, group);
        if alpha.is_none()
            && turned.is_none()
            && !(paint_rules::negative_inline_group(document, cascade, root, group)
                || paint_rules::inline_stacking_group(document, cascade, root, group)
                || paint_rules::negative_pseudo_group(document, cascade, root, group)
                || paint_rules::pseudo_stacking_group(document, cascade, root, group))
        {
            return; // cov:ignore: groups are found by their opacity or transform
        }
        if turned.is_some_and(paint_transform::is_singular) {
            return;
        }
        let turn = turned
            .zip(pseudo_box(items, root, group, part))
            .map(|(cv, rect)| paint_transform::box_transform(cv, rect));
        if let Some(turn) = turn {
            self.push_transform(turn);
        }
        if let Some(alpha) = alpha {
            self.push_opacity(alpha);
        }
        let mut content = Vec::new();
        let nested = push_paragraph(
            document,
            items,
            root,
            Some(group),
            lines,
            part,
            &mut content,
        );
        let nested = document.groups_with_content(items, root, nested, lines, part);
        let boxes = document.ifc_boxes_in_group(cascade, root, Some(group), part);
        let frames = document.paragraph_frames(cascade, root, boxes, nested, part, false);
        // The group's own boxes are its background, below the stacking
        // contexts nested in it.
        let own = own_box_events(group, &content);
        self.list_content(content, own, frames);
    }

    /// The opacity of inline group `group` of paragraph `root`, if it is
    /// below one.
    fn group_opacity(&self, root: usize, group: usize) -> Option<f32> {
        let letter = crate::generated_content::generated_origin(group)
            .is_some_and(|(_, pseudo)| pseudo == PseudoElem::FirstLetter);
        letter
            .then(|| self.items.letter_groups(root).opacity(group))
            .flatten()
            .or_else(|| {
                crate::generated_content::computed_for_id(self.cascade, group)
                    .and_then(paint_rules::opacity_layer)
            })
    }

    /// Visit node `node_id`: list its own events and push the frames of what
    /// is inside it.
    fn visit(&mut self, node_id: usize) {
        let (document, cascade, items, lines) =
            (self.document, self.cascade, self.items, self.lines);
        let page_index = self.page_index;
        // A subtree with nothing on this page lists no events.
        if self
            .entered
            .as_ref()
            .is_some_and(|entered| !entered.contains(&node_id))
        {
            return;
        }
        let Some(node) = document.get_node(node_id) else {
            return; // cov:ignore: node ids on the stack come from the arena
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() {
            return;
        }
        match node.kind() {
            NodeKind::Element => {
                if node.is_display_none() || node.is_hidden_by_text_overflow() {
                    return;
                }
                let Some(cv) = cascade.computed.get(node_id) else {
                    return; // cov:ignore: the cascade has a value for every arena node
                };
                // The transform stays in effect, like the group, until the
                // element's whole subtree has been listed.
                let space = self.space();
                // The painter places a fixed box in the page area apart from
                // the relative offsets around it; any transform around it
                // would contain it instead.
                if space != PaintTransform::IDENTITY
                    && self.motion.viewport_fixed(document, cascade, node_id)
                {
                    self.push_transform(PaintTransform::translate(-space.e, -space.f));
                }
                match paint_transform::element_motion(
                    document,
                    cascade,
                    &document.page_projection.run_context,
                    &mut self.motion,
                    items.page,
                    node_id,
                    cv,
                ) {
                    Motion::Still => {}
                    Motion::Moved(transform) => self.push_transform(transform),
                    Motion::Hidden => return,
                }
                // The group stays open until the element's whole subtree has
                // been listed.
                if let Some(alpha) = paint_rules::opacity_layer(cv) {
                    self.push_opacity(alpha);
                }
                let own: Vec<&PageFragmentItem> = items
                    .of(node_id)
                    .iter()
                    .copied()
                    .filter(|item| item.kind != PageFragmentKind::Text)
                    .collect();
                if paint_rules::generates_box(cv)
                    && !paint_rules::is_visibility_hidden_table(cv)
                    && !node.hides_empty_table_cell
                {
                    let replaced_content =
                        node.is_inline_svg_root() || document.is_canvas_element(node_id);
                    for &item in &own {
                        let fragment = items.fragment(item);
                        if let Some(cells) = document
                            .anonymous_table_part_background_cells(node_id)
                            // A part that is not visible has no cell
                            // backgrounds to clip; its box is dropped with
                            // the others below.
                            .filter(|_| !paint_rules::hides_own_box(cv))
                            .filter(|_| paint_rules::paints_table_part_decoration(cv))
                        {
                            // Box shadows are ignored on internal table
                            // elements only in the collapsing borders model
                            // (CSS Backgrounds 3 §7.1).
                            if cv.border_collapse
                                == raikiri_style::property::BorderCollapseValue::Separate
                                && cv.box_shadow.iter().any(|shadow| !shadow.inset)
                            {
                                self.events.push(PaintEvent::OuterBoxShadow(fragment));
                            }
                            let rect = fragment.paint_rect();
                            let top = rect.y - item.rect.y + item.box_y;
                            // The cells cover the whole part, but this
                            // fragment paints only inside its own slice: a
                            // cell outside it would clip away everything, and
                            // on a long table most cells are outside.
                            let in_slice = |clip: &PaintRect| {
                                clip.x < rect.x + rect.width
                                    && clip.y < rect.y + rect.height
                                    && clip.x + clip.width > rect.x
                                    && clip.y + clip.height > rect.y
                            };
                            for cell in cells {
                                let clip = PaintRect::new(
                                    rect.x + cell.x,
                                    top + cell.y,
                                    cell.width,
                                    cell.height,
                                );
                                if !in_slice(&clip) {
                                    continue;
                                }
                                self.events.push(PaintEvent::PushClip(
                                    PaintClip::new(clip),
                                    ClipKind::TableCell,
                                ));
                                self.events.push(PaintEvent::Box(fragment));
                                self.events.push(PaintEvent::PopClip);
                            }
                        } else {
                            self.events.push(PaintEvent::Box(fragment));
                        }
                        if item.kind == PageFragmentKind::Replaced || replaced_content {
                            if let Some(clip) = fragment.overflow_clip() {
                                self.events
                                    .push(PaintEvent::PushClip(clip, ClipKind::Overflow));
                            }
                            self.events.push(PaintEvent::Replaced(fragment));
                            if fragment.overflow_clip().is_some() {
                                self.events.push(PaintEvent::PopClip);
                            }
                        }
                    }
                }
                if let Some(rules) = document
                    .page_projection
                    .column_rules
                    .get(&page_index)
                    .and_then(|rules| rules.get(&NodeId::new(node_id as u64)))
                {
                    let clip = items.overflow_clips.get(&NodeId::new(node_id as u64));
                    if let Some(entry) = clip {
                        self.events
                            .push(PaintEvent::PushClip(entry.clip, ClipKind::Overflow));
                    }
                    self.events
                        .extend(rules.iter().copied().map(PaintEvent::ColumnRule));
                    if clip.is_some() {
                        self.events.push(PaintEvent::PopClip);
                    }
                }
                let image_marker = document
                    .page_marker_image(page_index, NodeId::new(node_id as u64))
                    .is_some();
                let inline_marker =
                    crate::generated_content::inside_marker_in_flow(cascade, node_id)
                        && node.is_ifc_root();
                if image_marker && !inline_marker {
                    self.events
                        .push(PaintEvent::MarkerImage(NodeId::new(node_id as u64)));
                }
                let mut clipped = false;
                if let Some(lines) = lines {
                    let marker = crate::generated_content::generated_node_id(
                        node_id,
                        raikiri_style::PseudoElem::Marker,
                    );
                    self.events.extend(
                        lines
                            .paragraphs
                            .get(&NodeId::new(marker as u64))
                            .into_iter()
                            .flatten()
                            .copied()
                            .map(PaintEvent::TextLine),
                    );
                }
                // The clip is the padding box of the element's whole box
                // resolved from shared source geometry. It opens after the
                // element's own box and closes after its subtree.
                if let Some(entry) = items.overflow_clips.get(&NodeId::new(node_id as u64)) {
                    // A closed vertical axis hides descendants on pages the
                    // box does not reach. An open vertical axis still admits
                    // them and must preserve the horizontal clip.
                    if own.is_empty() && entry.clip.clip_y {
                        return;
                    }
                    self.events
                        .push(PaintEvent::PushClip(entry.clip, ClipKind::Overflow));
                    self.stack.push(Frame::PopClip);
                    clipped = true;
                }
                // A plain block in a block container leaves its inline
                // content to the end of the layer (CSS 2.1 Appendix E), after
                // the backgrounds of the blocks that follow it.
                let deferring = self.layers.last().is_some_and(|layer| layer.allowed);
                let defers = deferring
                    && !clipped
                    && paint_rules::defers_inline_content(document, cascade, node_id);
                // A plain block that clips collects its inline content in a
                // layer of its own, which it hands to the enclosing layer
                // inside its clip.
                let merges = deferring
                    && clipped
                    && paint_rules::merges_clipped_inline_content(document, cascade, node_id);
                // In-flow blocks inside a paragraph's inline content paint
                // their backgrounds before its lines. With content positioned
                // below them, the paragraph's content lists them after that
                // content.
                let blocks: Vec<usize> = if node.is_ifc_root()
                    && !paint_rules::holds_negative_stack_level(document, cascade, node_id)
                {
                    document.paragraph_blocks(cascade, node_id)
                } else {
                    Vec::new()
                };
                if defers && node.is_ifc_root() {
                    if let Some(layer) = self.layers.last_mut() {
                        layer.pending.push(Frame::Content(node_id));
                    }
                    self.stack
                        .extend(blocks.into_iter().rev().map(Frame::Visit));
                    return;
                }
                if node.is_inline_svg_root()
                    || document.anonymous_table_contents_paint_only(node_id)
                {
                    self.stack
                        .extend(node.is_ifc_root().then_some(Frame::Content(node_id)));
                    return;
                }
                // A `display: contents` element has no box, so its children
                // share the enclosing layer.
                if !defers && paint_rules::generates_box(cv) {
                    let columns = node.is_multicol_container();
                    let close = if merges {
                        LayerClose::Merge(node_id)
                    } else if paint_rules::hoists_positioned_content(document, cascade, node_id) {
                        LayerClose::Hoist
                    } else {
                        LayerClose::Paint
                    };
                    self.layers.push(Layer::new(deferring && !columns, close));
                    self.stack.push(Frame::EndDeferral);
                }
                if node.is_ifc_root() {
                    // The paragraph's own text and inline elements are listed
                    // first; only the boxes laid out beside its lines are
                    // visited like ordinary children. Those in an inline
                    // opacity group are visited in the group.
                    if merges {
                        if let Some(layer) = self.layers.last_mut() {
                            layer.pending.push(Frame::Content(node_id));
                        }
                    } else {
                        self.stack.push(Frame::Content(node_id));
                    }
                    self.stack
                        .extend(blocks.into_iter().rev().map(Frame::Visit));
                    return;
                }
                let mut children =
                    if let Some(children) = document.anonymous_table_paint_sequence(node_id) {
                        children
                    } else {
                        node.children.clone()
                    };
                paint_rules::sort_paint_children(&mut children, node.display(), cascade);
                self.stack.extend(children.into_iter().rev().map(|key| {
                    match document.child_frame(key) {
                        Frame::Paragraph(key) if defers => Frame::DeferredParagraph(key),
                        frame => frame,
                    }
                }));
            }
            // A text node laid out as an anonymous flex or grid item is a
            // paragraph of its own. Any other text node outside a paragraph
            // has no lines to draw.
            NodeKind::Text if node.is_ifc_root() => {
                // A lone text node has no inline elements, so no groups.
                push_paragraph(
                    document,
                    items,
                    node_id,
                    None,
                    lines,
                    LinePart::All,
                    &mut self.events,
                );
            }
            _ => {} // cov:ignore: comments and other node kinds are out of the document
        }
    }
}
