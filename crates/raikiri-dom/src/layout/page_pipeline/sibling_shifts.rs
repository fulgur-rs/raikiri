//! Deferred moves of the boxes that follow a paginated column container.

use super::{Document, FragmentEdits, HashMap, LayoutError, ProjectionWork};
use crate::layout::context::LayoutSession;
use raikiri_layout_style::values::Position;

/// Moves of the siblings that follow each paginated column container and
/// its ancestors. A container that grows moves every later sibling at each
/// level; doing that for every container costs the container count times
/// the sibling count, so the moves are recorded per parent and applied once,
/// when the parent's children are read or at the end.
#[derive(Default)]
pub(super) struct SiblingShifts {
    /// For each parent, the moves in the order they were made.
    pending: HashMap<usize, Moves>,
    /// The position of each child in its parent's children, for the parents
    /// with moves.
    position: HashMap<usize, usize>,
}

/// The moves recorded for one parent: the children after the child at this
/// position move by this much.
struct Moves {
    events: Vec<(usize, f32)>,
    /// While every move starts at or after the one before it, the moves that
    /// reach a child are a prefix of `events`, and `running[count]` is the sum
    /// of the first `count` deltas, added in the order they were made.
    running: Vec<f32>,
    /// Whether the moves so far start in that order.
    in_order: bool,
}

impl Moves {
    fn new() -> Self {
        // An empty sum, whatever sign of zero the standard library gives it.
        Self {
            events: Vec::new(),
            running: vec![std::iter::empty::<f32>().sum()],
            in_order: true,
        }
    }

    fn push(&mut self, after: usize, delta: f32) {
        #[cfg(test)]
        super::tests::record_sibling_shift_work(0, 1);
        if self.in_order {
            match self.events.last() {
                Some(&(last, _)) if after < last => self.in_order = false,
                _ => {
                    let total = self.running[self.events.len()];
                    self.running.push(total + delta);
                }
            }
        }
        self.events.push((after, delta));
    }

    /// The sum of the deltas of the moves that start before the child at
    /// `index`, added in the order they were made.
    fn before(&self, index: usize) -> f32 {
        #[cfg(test)]
        super::tests::record_sibling_shift_work(1, 1);
        if self.in_order {
            let count = self.events.partition_point(|&(after, _)| {
                #[cfg(test)]
                super::tests::record_sibling_shift_work(2, 1);
                after < index
            });
            return self.running[count];
        }
        self.events
            .iter()
            .filter(|&&(after, _)| {
                #[cfg(test)]
                super::tests::record_sibling_shift_work(2, 1);
                after < index
            })
            .map(|&(_, delta)| delta)
            .sum()
    }
}

impl SiblingShifts {
    /// Move the children of `parent` after `child` by `delta`.
    pub(super) fn record(
        &mut self,
        document: &Document,
        parent: usize,
        child: usize,
        delta: f32,
        work: &mut ProjectionWork<'_, '_>,
    ) -> Result<(), LayoutError> {
        if !self.pending.contains_key(&parent) {
            let children = &document.nodes[parent].children;
            work.charge(children.len())?;
            self.position
                .extend(children.iter().enumerate().map(|(index, &id)| (id, index)));
        }
        let Some(&index) = self.position.get(&child) else {
            return Ok(()); // cov:ignore: a layout child is one of its parent's children.
        };
        self.pending
            .entry(parent)
            .or_insert_with(Moves::new)
            .push(index, delta);
        Ok(())
    }

    /// A following sibling moves with the flow unless it is out of flow or
    /// placed by its parent's paragraph.
    fn moves(document: &LayoutSession<'_>, parent: usize, sibling: usize) -> bool {
        let node = &document.nodes[sibling];
        node.layout_position() != Position::Absolute
            && !node.in_ifc_subtree()
            && !document.nodes[parent]
                .ifc
                .as_ref()
                .is_some_and(|root| root.boxes.iter().any(|b| b.node == sibling))
            && !document.pagination_style(sibling).float().is_floated()
    }

    /// The move recorded for `child` of `parent` and not applied yet.
    fn pending_for(&self, document: &LayoutSession<'_>, parent: usize, child: usize) -> f32 {
        let Some(moves) = self.pending.get(&parent) else {
            return 0.0;
        };
        let Some(&index) = self.position.get(&child) else {
            return 0.0; // cov:ignore: a layout child is one of its parent's children.
        };
        if !Self::moves(document, parent, child) {
            return 0.0;
        }
        moves.before(index)
    }

    /// The moves not applied yet to `node` and its layout ancestors: what
    /// their summed offsets lack.
    pub(super) fn offset(&self, document: &LayoutSession<'_>, node: usize) -> f32 {
        if self.pending.is_empty() {
            return 0.0;
        }
        let mut offset = 0.0;
        let mut id = node;
        for parent in document.layout_ancestors(node) {
            offset += self.pending_for(document, parent, id);
            id = parent;
        }
        offset
    }

    /// Apply the moves recorded for the children of `parent`.
    pub(super) fn apply(
        &mut self,
        document: &mut LayoutSession<'_>,
        edits: &mut FragmentEdits,
        parent: usize,
    ) {
        let Some(moves) = self.pending.remove(&parent) else {
            return;
        };
        let children = document.nodes[parent].children.clone();
        // Each move starts at the child after its position; a running sum
        // gives every child the moves before it.
        let mut starts = vec![0.0_f32; children.len() + 1];
        for (after, delta) in moves.events {
            starts[after + 1] += delta;
        }
        let mut shifted = HashMap::new();
        let mut delta = 0.0_f32;
        for (index, &child) in children.iter().enumerate() {
            delta += starts[index];
            if delta != 0.0 && Self::moves(document, parent, child) {
                document.nodes[child].layout.location.y += delta;
                shifted.insert(child, delta);
            }
        }
        edits.shift_following(parent, &shifted);
    }

    /// Apply every recorded move.
    pub(super) fn apply_all(
        &mut self,
        document: &mut LayoutSession<'_>,
        edits: &mut FragmentEdits,
    ) {
        let parents: Vec<usize> = self.pending.keys().copied().collect();
        for parent in parents {
            self.apply(document, edits, parent);
        }
    }
}
