//! Reorder balanced event slices at their actual stacking-context boundary.

use super::*;

pub(super) struct Context<'a> {
    id: usize,
    start: usize,
    depth: usize,
    boxes: Vec<((i32, usize), Vec<PaintEvent<'a>>)>,
}

pub(super) struct BoxCapture<'a> {
    owner: usize,
    key: (i32, usize),
    start: usize,
    prefix: Vec<PaintEvent<'a>>,
}

fn close(event: PaintEvent<'_>) -> PaintEvent<'_> {
    match event {
        PaintEvent::PushClip(..) => PaintEvent::PopClip,
        PaintEvent::PushTransform(_) => PaintEvent::PopTransform,
        PaintEvent::PushOpacity(_) => PaintEvent::PopOpacity,
        _ => unreachable!("only open scopes are kept"),
    }
}

impl<'a> PaintWalk<'a, '_> {
    pub(super) fn begin_negative_box(&mut self, id: usize) {
        self.capture_negative_box(id, self.negative_levels.first_line_owner(id));
    }

    pub(super) fn begin_negative_group(&mut self, id: usize, first_line: bool) {
        let owner = first_line
            .then(|| self.negative_levels.first_line_group_owner(id))
            .flatten();
        self.capture_negative_box(id, owner);
    }

    fn capture_negative_box(&mut self, id: usize, owner_override: Option<usize>) {
        let Some((owner, level, rank)) = self.negative_levels.box_key(id) else {
            return;
        };
        let owner = owner_override.unwrap_or(owner);
        let Some(context) = self
            .negative_contexts
            .last_mut()
            .filter(|context| context.id == owner)
        else {
            return;
        };
        self.negative_boxes.push(BoxCapture {
            owner,
            key: (level, rank),
            start: self.events.len(),
            prefix: self.scopes[context.depth..].to_vec(),
        });
        self.stack.push(Frame::EndNegative);
    }

    pub(super) fn end_negative_box(&mut self) {
        let captured = self.negative_boxes.pop().expect("a negative box capture");
        let mut events = captured.prefix.clone();
        events.extend(self.events.split_off(captured.start));
        events.extend(captured.prefix.into_iter().rev().map(close));
        let context = self
            .negative_contexts
            .last_mut()
            .expect("an owning context");
        assert_eq!(context.id, captured.owner);
        context.boxes.push((captured.key, events));
    }

    pub(super) fn begin_negative_context(&mut self, id: usize) {
        if !self.negative_levels.holds(id) {
            return;
        }
        self.open_negative_context(id);
    }

    pub(super) fn begin_first_line_negative_context(&mut self, root: usize) {
        if self.negative_levels.holds_first_line(root) {
            self.open_negative_context(root);
        }
    }

    fn open_negative_context(&mut self, id: usize) {
        self.negative_contexts.push(Context {
            id,
            start: self.events.len(),
            depth: self.scopes.len(),
            boxes: Vec::new(),
        });
        self.stack.push(Frame::EndNegativeContext(id));
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
        let normal = self.events.split_off(context.start);
        context.boxes.sort_by_key(|(key, _)| *key);
        for (_, events) in context.boxes {
            self.events.extend(events);
        }
        self.events.extend(normal);
    }
}
