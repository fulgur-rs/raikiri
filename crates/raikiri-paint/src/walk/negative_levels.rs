//! Balanced scene captures for the negative layer of a stacking context.

use super::frames::PaintFrame;
use super::paint_walk::PaintWalk;
use crate::PaintTraceEvent;
use anyrender::{PaintScene, Scene};
use kurbo::Affine;
use std::collections::HashMap;

pub(super) struct Context {
    id: usize,
    clip_depth: usize,
    trace_start: Option<usize>,
    trace_depth: usize,
    boxes: Vec<((i32, usize), Scene, Vec<PaintTraceEvent>)>,
}

pub(super) struct BoxCapture {
    node: usize,
    owner: usize,
    key: (i32, usize),
    prefix: Scene,
    clips: usize,
    trace_prefix: Vec<PaintTraceEvent>,
    trace_start: Option<usize>,
    inline: Option<(usize, usize)>,
    first_line: bool,
}

pub(super) struct InlineBoxScope {
    pub(super) root: usize,
    pub(super) node: usize,
    pub(super) clip_depth: usize,
    trace_depth: usize,
}

pub(super) struct InlineTrace {
    pub(super) start: usize,
    boxes: Vec<(usize, (i32, usize), Vec<PaintTraceEvent>)>,
}

enum TracePart {
    Event(PaintTraceEvent),
    Group(usize),
}

struct TraceGroup {
    owner: usize,
    alpha: f32,
    parts: Vec<TracePart>,
}

fn push_trace_part(
    roots: &mut Vec<TracePart>,
    groups: &mut [TraceGroup],
    active: &[usize],
    part: TracePart,
) {
    match active.last() {
        Some(&group) => groups[group].parts.push(part),
        None => roots.push(part),
    }
}

// Inline groups composite once even when several atomic boxes join them.
// Build their trace in that same order, without replaying their opacity.
fn inline_trace(
    document: &raikiri_dom::Document,
    normal: Vec<PaintTraceEvent>,
    negative: Vec<(usize, (i32, usize), Vec<PaintTraceEvent>)>,
) -> Vec<PaintTraceEvent> {
    let mut pending = HashMap::<usize, Vec<PaintTraceEvent>>::new();
    for (owner, _, events) in negative {
        pending.entry(owner).or_default().extend(events);
    }
    let mut roots = Vec::new();
    let mut groups = Vec::<TraceGroup>::new();
    let mut by_owner = HashMap::new();
    let mut active = Vec::new();
    for event in normal {
        match event {
            PaintTraceEvent::PushOpacity(owner, alpha) => {
                let group = if let Some(&group) = by_owner.get(&owner) {
                    group
                } else {
                    let group = groups.len();
                    groups.push(TraceGroup {
                        owner,
                        alpha,
                        parts: pending
                            .remove(&owner)
                            .unwrap_or_default()
                            .into_iter()
                            .map(TracePart::Event)
                            .collect(),
                    });
                    by_owner.insert(owner, group);
                    push_trace_part(&mut roots, &mut groups, &active, TracePart::Group(group));
                    group
                };
                active.push(group);
            }
            PaintTraceEvent::PopOpacity => {
                active.pop();
            }
            event => {
                if !pending.is_empty()
                    && let PaintTraceEvent::Box(id) = event
                {
                    for owner in document.inclusive_ancestors(id) {
                        if let Some(events) = pending.remove(&owner) {
                            for event in events {
                                push_trace_part(
                                    &mut roots,
                                    &mut groups,
                                    &active,
                                    TracePart::Event(event),
                                );
                            }
                        }
                    }
                }
                push_trace_part(&mut roots, &mut groups, &active, TracePart::Event(event));
            }
        }
    }
    let mut output = Vec::new();
    let mut stack: Vec<_> = roots.into_iter().rev().collect();
    while let Some(part) = stack.pop() {
        match part {
            TracePart::Event(event) => output.push(event),
            TracePart::Group(index) => {
                let group = &mut groups[index];
                output.push(PaintTraceEvent::PushOpacity(group.owner, group.alpha));
                stack.push(TracePart::Event(PaintTraceEvent::PopOpacity));
                stack.extend(std::mem::take(&mut group.parts).into_iter().rev());
            }
        }
    }
    output
}

impl<S: PaintScene> PaintWalk<'_, S> {
    pub(super) fn begin_negative_box(&mut self, id: usize) {
        let Some((owner, level, rank)) = self.negative_levels.box_key(id) else {
            return;
        };
        let first_line = self.negative_levels.first_line_owner(id);
        let owner = first_line.unwrap_or(owner);
        let context = self
            .negative_contexts
            .last_mut()
            .filter(|context| first_line.is_none() && context.id == owner);
        let (depth, trace_prefix, inline) = if let Some(context) = context {
            (
                context.clip_depth,
                self.trace_clips[context.trace_depth..].to_vec(),
                None,
            )
        } else if let Some(scope) = self.inline_boxes.last_mut().filter(|scope| {
            (first_line.is_some()
                || (self.negative_levels.holds(owner)
                    && self
                        .cascade
                        .computed
                        .get(owner)
                        .is_some_and(|cv| cv.display.is_inline_flow())))
                && self
                    .document
                    .inclusive_ancestors(scope.node)
                    .any(|id| id == owner)
                && self
                    .held_lines
                    .iter()
                    .any(|lines| lines.root_id() == scope.root)
        }) {
            (
                scope.clip_depth,
                self.trace_clips[scope.trace_depth..].to_vec(),
                Some((scope.root, scope.node)),
            )
        } else {
            return;
        };
        let (prefix, clips) = self.scene.scene.clip_prefix(depth);
        self.negative_boxes.push(BoxCapture {
            node: id,
            owner,
            key: (level, rank),
            prefix,
            clips,
            trace_prefix,
            trace_start: self.trace.as_deref().map(Vec::len),
            inline,
            first_line: first_line.is_some(),
        });
        self.scene.scene.begin_capture();
        self.stack.push(PaintFrame::EndNegative);
    }

    pub(super) fn end_negative_box(&mut self) {
        let mut captured = self.negative_boxes.pop().expect("a negative box capture");
        captured
            .prefix
            .append_scene(self.scene.scene.end_capture(), Affine::IDENTITY);
        for _ in 0..captured.clips {
            captured.prefix.pop_layer();
        }
        let mut trace = captured.trace_prefix;
        let clips = trace.len();
        if let Some((start, events)) = captured.trace_start.zip(self.trace.as_deref_mut()) {
            trace.extend(events.split_off(start));
        }
        trace.extend((0..clips).map(|_| PaintTraceEvent::PopClip));
        if let Some((root, atomic)) = captured.inline {
            self.held_lines
                .iter_mut()
                .rev()
                .find(|lines| lines.root_id() == root)
                .expect("the owning held paragraph")
                .place_negative_box(
                    (!captured.first_line).then_some(captured.owner),
                    atomic,
                    captured.node,
                    captured.key,
                    captured.prefix,
                );
            if let Some(traces) = self.inline_traces.get_mut(&root) {
                traces.boxes.push((captured.owner, captured.key, trace));
            }
            return;
        }
        let context = self
            .negative_contexts
            .last_mut()
            .expect("an owning context");
        assert_eq!(context.id, captured.owner);
        context.boxes.push((captured.key, captured.prefix, trace));
    }

    pub(super) fn start_inline_box(&mut self, root: usize, node: usize, start: usize) {
        if self.negative_levels.is_empty() {
            return;
        }
        self.inline_traces.entry(root).or_insert(InlineTrace {
            start,
            boxes: Vec::new(),
        });
        self.inline_boxes.push(InlineBoxScope {
            root,
            node,
            clip_depth: self.scene.scene.layer_depth(),
            trace_depth: self.trace_clips.len(),
        });
    }

    pub(super) fn finish_inline_negative_trace(&mut self, root: usize) {
        let Some(mut captured) = self.inline_traces.remove(&root) else {
            return;
        };
        let Some(trace) = self.trace.as_deref_mut() else {
            return;
        };
        captured.boxes.sort_by_key(|(_, key, _)| *key);
        if !captured.boxes.is_empty() {
            let normal = trace.split_off(captured.start);
            trace.extend(inline_trace(self.document, normal, captured.boxes));
        }
    }

    pub(super) fn begin_negative_context(&mut self, id: usize) {
        if !self.negative_levels.holds(id) {
            return;
        }
        self.negative_contexts.push(Context {
            id,
            clip_depth: self.scene.scene.layer_depth(),
            trace_start: self.trace.as_deref().map(Vec::len),
            trace_depth: self.trace_clips.len(),
            boxes: Vec::new(),
        });
        self.scene.scene.begin_capture();
    }

    pub(super) fn end_negative_context(&mut self, id: usize) {
        if self
            .negative_contexts
            .last()
            .is_none_or(|context| context.id != id)
        {
            return;
        }
        let mut context = self.negative_contexts.pop().unwrap();
        debug_assert_eq!(self.scene.scene.layer_depth(), context.clip_depth);
        let normal = self.scene.scene.end_capture();
        let normal_trace = context
            .trace_start
            .zip(self.trace.as_deref_mut())
            .map(|(start, trace)| trace.split_off(start));
        context.boxes.sort_by_key(|(key, _, _)| *key);
        for (_, scene, events) in context.boxes {
            self.scene.scene.append_scene(scene, Affine::IDENTITY);
            if let Some(trace) = self.trace.as_deref_mut() {
                trace.extend(events);
            }
        }
        self.scene.scene.append_scene(normal, Affine::IDENTITY);
        if let Some((normal, trace)) = normal_trace.zip(self.trace.as_deref_mut()) {
            trace.extend(normal);
        }
    }
}
