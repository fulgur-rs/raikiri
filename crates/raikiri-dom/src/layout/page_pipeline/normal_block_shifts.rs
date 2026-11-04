//! Deferred local translations and content extents during block pagination.

use std::collections::{BinaryHeap, HashMap, HashSet};

#[cfg(test)]
thread_local! {
    static FORCE_EAGER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DEPTH_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static FALLBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Independent maxima for in-flow, border-box and exported float ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ends(pub(crate) [f32; 3]);

impl Ends {
    pub(crate) const EMPTY: Self = Self([f32::NEG_INFINITY; 3]);

    #[cfg(test)]
    fn translated(self, delta: f32) -> Self {
        Self(self.0.map(|end| end + delta))
    }

    #[cfg(test)]
    fn max(self, other: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i].max(other.0[i])))
    }
}

// Wide arithmetic reduces regrouping error in deferred additions. Reads and
// final materialization share one rounding boundary. The phase falls back to
// eager updates when the conservative roundoff estimate becomes significant.
#[derive(Clone, Copy, Debug)]
struct WideEnds([f64; 3]);

impl WideEnds {
    const EMPTY: Self = Self([f64::NEG_INFINITY; 3]);

    fn translated(self, delta: f64) -> Self {
        Self(self.0.map(|end| end + delta))
    }

    fn max(self, other: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i].max(other.0[i])))
    }

    fn narrow(self) -> Ends {
        Ends(self.0.map(|end| end as f32))
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    moving: WideEnds,
    anchored: WideEnds,
    lazy: f64,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            moving: WideEnds::EMPTY,
            anchored: WideEnds::EMPTY,
            lazy: 0.0,
        }
    }
}

/// A child's extents and whether it follows a preceding block's movement.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Leaf {
    pub(crate) node: usize,
    pub(crate) ends: Ends,
    pub(crate) follows: bool,
    pub(crate) float_follows: bool,
}

/// Range additions preserve anchored children and all three extent maxima.
#[derive(Clone, Debug)]
struct Parent {
    leaves: Vec<Leaf>,
    entries: Vec<Entry>,
    last_flow: Option<usize>,
}

impl Parent {
    fn new(leaves: Vec<Leaf>) -> Self {
        let last_flow = leaves
            .iter()
            .rev()
            .find(|leaf| leaf.ends.0[0].is_finite())
            .map(|leaf| leaf.node);
        let mut result = Self {
            last_flow,
            entries: vec![Entry::default(); leaves.len().saturating_mul(4)],
            leaves,
        };
        if !result.leaves.is_empty() {
            result.build(1, 0, result.leaves.len());
        }
        result
    }

    fn build(&mut self, entry: usize, start: usize, end: usize) {
        if end - start == 1 {
            self.write_leaf(entry, start, self.leaves[start].ends);
        } else {
            let middle = (start + end) / 2;
            self.build(entry * 2, start, middle);
            self.build(entry * 2 + 1, middle, end);
            self.pull(entry);
        }
    }

    fn write_leaf(&mut self, entry: usize, index: usize, ends: Ends) {
        let slot = &mut self.entries[entry];
        for channel in 0..3 {
            let follows = if channel == 2 {
                self.leaves[index].float_follows
            } else {
                self.leaves[index].follows
            };
            if follows {
                slot.moving.0[channel] = f64::from(ends.0[channel]);
            } else {
                slot.anchored.0[channel] = f64::from(ends.0[channel]);
            }
        }
    }

    fn pull(&mut self, entry: usize) {
        self.entries[entry].moving = self.entries[entry * 2]
            .moving
            .max(self.entries[entry * 2 + 1].moving);
        self.entries[entry].anchored = self.entries[entry * 2]
            .anchored
            .max(self.entries[entry * 2 + 1].anchored);
    }

    fn add_entry(&mut self, entry: usize, delta: f64) {
        self.entries[entry].moving = self.entries[entry].moving.translated(delta);
        self.entries[entry].lazy += delta;
    }

    fn push(&mut self, entry: usize) {
        let delta = self.entries[entry].lazy;
        if delta != 0.0 {
            self.add_entry(entry * 2, delta);
            self.add_entry(entry * 2 + 1, delta);
            self.entries[entry].lazy = 0.0;
        }
    }

    fn add(&mut self, entry: usize, start: usize, end: usize, from: usize, delta: f64) {
        if end <= from {
            return;
        }
        if from <= start {
            self.add_entry(entry, delta);
            return;
        }
        self.push(entry);
        let middle = (start + end) / 2;
        self.add(entry * 2, start, middle, from, delta);
        self.add(entry * 2 + 1, middle, end, from, delta);
        self.pull(entry);
    }

    fn pending(&self, index: usize) -> f64 {
        if !self.leaves[index].follows {
            return 0.0;
        }
        let (mut entry, mut start, mut end) = (1, 0, self.leaves.len());
        let mut delta = 0.0;
        loop {
            delta += self.entries[entry].lazy;
            if end - start == 1 {
                return delta;
            }
            let middle = (start + end) / 2;
            entry *= 2;
            if index < middle {
                end = middle;
            } else {
                start = middle;
                entry += 1;
            }
        }
    }

    fn consume(&mut self, entry: usize, start: usize, end: usize, index: usize) {
        if end - start == 1 {
            // Consuming an offset changes representation, not the extents.
            self.entries[entry].lazy = 0.0;
            return;
        }
        self.push(entry);
        let middle = (start + end) / 2;
        if index < middle {
            self.consume(entry * 2, start, middle, index);
        } else {
            self.consume(entry * 2 + 1, middle, end, index);
        }
    }

    fn set(&mut self, entry: usize, start: usize, end: usize, index: usize, ends: Ends) {
        if end - start == 1 {
            self.write_leaf(entry, index, ends);
            return;
        }
        self.push(entry);
        let middle = (start + end) / 2;
        if index < middle {
            self.set(entry * 2, start, middle, index, ends);
        } else {
            self.set(entry * 2 + 1, middle, end, index, ends);
        }
        self.pull(entry);
    }

    fn query(&self, range: std::ops::Range<usize>) -> Ends {
        self.query_entry(1, 0, self.leaves.len(), &range, 0.0)
            .narrow()
    }

    fn query_entry(
        &self,
        entry: usize,
        start: usize,
        end: usize,
        range: &std::ops::Range<usize>,
        inherited: f64,
    ) -> WideEnds {
        if end <= range.start || range.end <= start || start == end {
            return WideEnds::EMPTY;
        }
        let slot = self.entries[entry];
        if range.start <= start && end <= range.end {
            return slot.moving.translated(inherited).max(slot.anchored);
        }
        let middle = (start + end) / 2;
        let inherited = inherited + slot.lazy;
        self.query_entry(entry * 2, start, middle, range, inherited)
            .max(self.query_entry(entry * 2 + 1, middle, end, range, inherited))
    }
}

/// Phase-local indexes; layout keys need not be dense source-node indices.
#[derive(Clone, Debug, Default)]
pub(crate) struct NormalBlockShifts {
    parents: HashMap<usize, Parent>,
    positions: HashMap<usize, (usize, usize)>,
    dirty: HashSet<usize>,
    dirty_order: BinaryHeap<(usize, usize)>,
    depths: HashMap<usize, usize>,
    maximum_input: f64,
    absolute_deltas: f64,
    quantum_exponent: Option<i32>,
    arithmetic_updates: f64,
}

/// Exponent of the greatest power-of-two step dividing a finite nonzero f32.
fn binary_quantum(value: f32) -> Option<i32> {
    if !value.is_finite() || value == 0.0 {
        return None;
    }
    let bits = value.abs().to_bits();
    let exponent = (bits >> 23) as i32;
    let fraction = bits & 0x7f_ffff;
    let (significand, scale) = if exponent == 0 {
        (fraction, -149)
    } else {
        (fraction | (1 << 23), exponent - 150)
    };
    Some(scale + significand.trailing_zeros() as i32)
}

/// Upper bound for a sum of nonnegative magnitudes, including rounding.
fn upper_sum(left: f64, right: f64) -> f64 {
    if right == 0.0 {
        left
    } else {
        (left + right).next_up()
    }
}

impl NormalBlockShifts {
    pub(crate) fn insert(&mut self, parent: usize, leaves: Vec<Leaf>) {
        debug_assert!(!self.parents.contains_key(&parent));
        for (index, leaf) in leaves.iter().enumerate() {
            self.observe_magnitudes(leaf.ends.0);
            debug_assert!(!self.positions.contains_key(&leaf.node));
            self.positions.insert(leaf.node, (parent, index));
        }
        self.parents.insert(parent, Parent::new(leaves));
    }

    fn observe_quantum(&mut self, value: f32) {
        self.quantum_exponent = self
            .quantum_exponent
            .into_iter()
            .chain(binary_quantum(value))
            .min();
    }

    fn observe_magnitudes(&mut self, values: [f32; 3]) {
        for value in values.into_iter().filter(|value| value.is_finite()) {
            self.maximum_input = self.maximum_input.max(f64::from(value).abs());
            self.observe_quantum(value);
        }
    }

    fn observe_geometry(&mut self, values: [f32; 3]) {
        if values.iter().any(|value| !value.is_finite()) {
            self.maximum_input = f64::INFINITY;
        }
        self.observe_magnitudes(values);
    }

    /// Exact binary-grid sums need no update-count limit. Otherwise keep the
    /// estimated wide-arithmetic error below pagination's 0.001px comparisons.
    /// Neither case promises bitwise identity with sequential f32 additions.
    pub(crate) fn can_defer(&self, delta: f32) -> bool {
        let magnitude = upper_sum(
            upper_sum(self.maximum_input, self.absolute_deltas),
            f64::from(delta).abs(),
        );
        if !delta.is_finite() || !magnitude.is_finite() || magnitude > f64::from(f32::MAX) {
            return false;
        }
        let quantum = self
            .quantum_exponent
            .into_iter()
            .chain(binary_quantum(delta))
            .min();
        // All operands are multiples of the shared step. Below 2^53 steps,
        // every intermediate sum is exactly representable in f64.
        if quantum.is_none_or(|exponent| magnitude < 2.0_f64.powi(exponent + 53)) {
            return true;
        }
        // Allow for both pending and maximum paths through up to 64 levels.
        let operations = 128.0 * (self.arithmetic_updates + 1.0) + 128.0;
        f64::EPSILON * operations * magnitude < 0.0001
    }

    pub(crate) fn mark_dirty(&mut self, node: usize) {
        if self.dirty.insert(node) {
            self.dirty_order
                .push((self.depths.get(&node).copied().unwrap_or(0), node));
        }
    }

    fn pop_deeper_than(&mut self, depth: usize) -> Option<usize> {
        if self.dirty_order.peek()?.0 <= depth {
            return None;
        }
        let (_, node) = self.dirty_order.pop().unwrap();
        self.dirty.remove(&node);
        #[cfg(test)]
        DEPTH_WORK.with(|work| work.set(work.get() + 1));
        Some(node)
    }

    pub(crate) fn position(&self, node: usize) -> Option<(usize, usize)> {
        self.positions.get(&node).copied()
    }

    pub(crate) fn len(&self, parent: usize) -> Option<usize> {
        self.parents.get(&parent).map(|parent| parent.leaves.len())
    }

    fn last_flow(&self, parent: usize) -> Option<usize> {
        self.parents.get(&parent)?.last_flow
    }

    pub(crate) fn pending(&self, node: usize) -> f32 {
        self.positions.get(&node).map_or(0.0, |&(parent, index)| {
            self.parents[&parent].pending(index) as f32
        })
    }

    pub(crate) fn consume(&mut self, node: usize) -> f32 {
        let Some(&(parent, index)) = self.positions.get(&node) else {
            return 0.0;
        };
        let parent = self.parents.get_mut(&parent).unwrap();
        let delta = parent.pending(index);
        parent.consume(1, 0, parent.leaves.len(), index);
        delta as f32
    }

    pub(crate) fn local_y(&self, node: usize, raw: f32) -> f32 {
        let Some(&(parent, index)) = self.positions.get(&node) else {
            return raw;
        };
        let pending = self.parents[&parent].pending(index);
        if pending == 0.0 {
            raw
        } else {
            (f64::from(raw) + pending) as f32
        }
    }

    pub(crate) fn shift_after(&mut self, parent: usize, child: usize, delta: f32) -> bool {
        let Some(&(owner, index)) = self.positions.get(&child) else {
            return false;
        };
        if owner != parent || !self.can_defer(delta) {
            return false;
        }
        self.observe_quantum(delta);
        self.absolute_deltas = upper_sum(self.absolute_deltas, f64::from(delta).abs());
        self.arithmetic_updates += 1.0;
        let parent = self.parents.get_mut(&parent).unwrap();
        if delta != 0.0 && delta.is_finite() {
            parent.add(1, 0, parent.leaves.len(), index + 1, f64::from(delta));
        }
        true
    }

    pub(crate) fn set_ends(&mut self, node: usize, ends: Ends) {
        if let Some(&(parent, index)) = self.positions.get(&node) {
            self.observe_magnitudes(ends.0);
            self.arithmetic_updates += 1.0;
            let parent = self.parents.get_mut(&parent).unwrap();
            parent.set(1, 0, parent.leaves.len(), index, ends);
        }
    }

    pub(crate) fn range(&self, parent: usize, range: std::ops::Range<usize>) -> Option<Ends> {
        self.parents.get(&parent).map(|parent| parent.query(range))
    }
}

impl crate::Document {
    /// Local pagination coordinate before any out-of-flow owner correction.
    pub(crate) fn pagination_local_y(&self, node: usize) -> f32 {
        let raw = self.nodes[node].layout.location.y;
        self.normal_block_shifts
            .as_ref()
            .map_or(raw, |state| state.local_y(node, raw))
    }

    /// Changes a coordinate after consuming its deferred representation.
    pub(crate) fn set_pagination_local_y(&mut self, node: usize, y: f32) {
        if let Some(state) = self.normal_block_shifts.as_mut() {
            let pending = state.consume(node);
            #[cfg(test)]
            if pending != 0.0 {
                super::tests::record_normal_sibling_work(1, 1);
            }
            #[cfg(not(test))]
            let _ = pending;
        }
        self.table_layout_node_mut(node).layout.location.y = y;
        self.mark_pagination_extent_dirty(node);
    }

    pub(crate) fn translate_pagination_y(&mut self, node: usize, delta: f32) {
        let y = self.pagination_local_y(node);
        self.set_pagination_local_y(node, y + delta);
    }

    pub(crate) fn mark_pagination_extent_dirty(&mut self, node: usize) {
        let source = self.ifc_source_owner(node);
        let layout = &self.nodes[node].layout;
        let magnitudes = [layout.location.y, layout.size.height, layout.margin.bottom];
        if let Some(state) = self.normal_block_shifts.as_mut() {
            state.observe_geometry(magnitudes);
            if state.can_defer(0.0) {
                state.mark_dirty(source);
            } else {
                self.abandon_normal_block_shifts();
            }
        }
    }

    pub(crate) fn abandon_normal_block_shifts(&mut self) {
        #[cfg(test)]
        FALLBACKS.with(|count| count.set(count.get() + 1));
        self.finish_normal_block_shifts();
    }

    /// Retains geometry on every exit from the candidate phase.
    pub(crate) fn finish_normal_block_shifts(&mut self) {
        let Some(state) = self.normal_block_shifts.take() else {
            return;
        };
        for &node in state.positions.keys() {
            let delta = state.pending(node);
            if delta != 0.0 {
                #[cfg(test)]
                super::tests::record_normal_sibling_work(1, 1);
                let y = state.local_y(node, self.nodes[node].layout.location.y);
                self.table_layout_node_mut(node).layout.location.y = y;
            }
        }
    }
}

#[cfg(test)]
mod tests;

use super::super::context::LayoutSession;
use super::{CascadeResult, DisplayValue, FloatValue, NodeKind, Position, PositionValue};

/// Indexes the stable child order after the pagination prepasses.
pub(super) fn begin(document: &mut LayoutSession<'_>, cascade: &CascadeResult, body: usize) {
    #[cfg(test)]
    if FORCE_EAGER.with(std::cell::Cell::get) {
        return;
    }
    debug_assert!(document.normal_block_shifts.is_none());
    let mut arithmetic = NormalBlockShifts::default();
    let mut depths = HashMap::new();
    let mut pending: Vec<_> = document
        .nodes
        .iter()
        .enumerate()
        .filter(|(id, _)| document.parent_of(*id).is_none())
        .map(|(id, _)| (id, 0))
        .collect();
    while let Some((node, depth)) = pending.pop() {
        if depths.insert(node, depth).is_some() {
            continue; // cov:ignore: defensive duplicate-node guard for the source-tree invariant.
        }
        let layout = &document.nodes[node].layout;
        arithmetic.observe_geometry([layout.location.y, layout.size.height, layout.margin.bottom]);
        #[cfg(test)]
        DEPTH_WORK.with(|work| work.set(work.get() + 1));
        pending.extend(
            document.nodes[node]
                .children
                .iter()
                .map(|&child| (child, depth + 1)),
        );
    }
    let mut parents: Vec<_> = document
        .nodes
        .iter()
        .enumerate()
        .filter(|(id, _)| super::moves_later_siblings(document, *id))
        .map(|(id, _)| (depths[&id], id))
        .collect();
    parents.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.0));
    arithmetic.depths = depths;
    document.normal_block_shifts = Some(Box::new(arithmetic));
    for (_, parent) in parents {
        let mut children = Vec::new();
        let ordered = super::pagination_child_order_borrowed(document, cascade, parent);
        let mut pending: Vec<_> = ordered.iter().rev().copied().collect();
        while let Some(child) = pending.pop() {
            #[cfg(test)]
            super::tests::record_normal_sibling_work(0, 1);
            if document.nodes[child].display() == DisplayValue::Contents {
                pending.extend(document.nodes[child].children.iter().rev().copied());
            } else {
                children.push(child);
            }
        }
        let mut leaves = Vec::with_capacity(children.len());
        for child in children {
            let Some(ends) = leaf_ends(document, cascade, body, parent, child) else {
                document.abandon_normal_block_shifts();
                return;
            };
            let follows = document.nodes[child].kind() == NodeKind::Element
                && document.nodes[child].display() != DisplayValue::None
                && !matches!(
                    document.nodes[child].position(),
                    PositionValue::Absolute | PositionValue::Fixed
                );
            leaves.push(Leaf {
                node: child,
                ends,
                follows,
                float_follows: follows
                    && (super::is_floating_box_for_pagination(document, cascade, child)
                        || document.contributes_layout_offset(child)),
            });
        }
        document
            .normal_block_shifts
            .as_mut()
            .unwrap()
            .insert(parent, leaves);
        if !document
            .normal_block_shifts
            .as_ref()
            .unwrap()
            .can_defer(0.0)
        {
            document.abandon_normal_block_shifts();
            return;
        }
    }
}

/// Finite ends and deliberate empty channels, or a derived numeric overflow.
fn leaf_ends(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body: usize,
    parent: usize,
    child: usize,
) -> Option<Ends> {
    #[cfg(test)]
    super::tests::record_normal_sibling_work(2, 1);
    let node = &document.nodes[child];
    if node.kind() == NodeKind::Text
        || node.display() == DisplayValue::None
        || node.layout_position() == Position::Absolute
    {
        return Some(Ends::EMPTY);
    }
    let layout = &node.layout;
    let top = super::pagination_child_parent_y(
        document,
        parent,
        child,
        document.pagination_local_y(child),
    );
    if super::is_floating_box_for_pagination(document, cascade, child) {
        return if matches!(
            cascade.computed[child].float,
            FloatValue::Left | FloatValue::Right
        ) {
            let end = top + layout.size.height + layout.margin.bottom;
            end.is_finite()
                .then_some(Ends([f32::NEG_INFINITY, f32::NEG_INFINITY, end]))
        } else {
            Some(Ends::EMPTY)
        };
    }
    let end =
        top + layout.size.height - super::layout_relative_offset_y(document, cascade, body, child);
    if !end.is_finite() {
        return None;
    }
    let float_end = branch_float_end(document, cascade, body, child, top)?;
    Some(Ends([end, end, float_end]))
}

fn branch_float_end(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body: usize,
    child: usize,
    top: f32,
) -> Option<f32> {
    if super::holds_floats(document, cascade, child)
        // cov:ignore: abs/fixed hold floats above; running positions are normalized by cascade.
        || !matches!(document.nodes[child].position(), PositionValue::Static | PositionValue::Relative | PositionValue::Sticky)
        || (cascade.computed[child].float == FloatValue::Footnote && super::is_floating_box_for_pagination(document, cascade, child))
    {
        return Some(f32::NEG_INFINITY);
    }
    if let Some(state) = document.normal_block_shifts.as_ref()
        && let Some(len) = state.len(child)
    {
        let end = state.range(child, 0..len).unwrap().0[2];
        if end == f32::NEG_INFINITY {
            return Some(end);
        }
        let correction = if document.contributes_layout_offset(child) {
            super::super::ifc::outside_marker::box_leading(document, child)
        } else {
            document.pagination_local_y(child)
        };
        let projected = end + top
            - super::layout_relative_offset_y(document, cascade, body, child)
            - correction;
        return projected.is_finite().then_some(projected);
    }
    let mut end = f32::NEG_INFINITY;
    let finite = super::visit_same_bfc_floats_in_subtree(
        document,
        cascade,
        body,
        child,
        top,
        |_, bottom| end = end.max(bottom),
    );
    finite.then_some(end)
}

/// Refreshes only branches changed by explicit geometry writes, deepest first.
pub(super) fn refresh(
    document: &mut LayoutSession<'_>,
    cascade: &CascadeResult,
    body: usize,
    parent: usize,
) {
    let Some(state) = document.normal_block_shifts.as_ref() else {
        return;
    };
    let depth = state.depths.get(&parent).copied().unwrap_or(0);
    while let Some(node) = document
        .normal_block_shifts
        .as_mut()
        .unwrap()
        .pop_deeper_than(depth)
    {
        let position = document
            .normal_block_shifts
            .as_ref()
            .unwrap()
            .position(node);
        if let Some((owner, _)) = position {
            let Some(ends) = leaf_ends(document, cascade, body, owner, node) else {
                document.abandon_normal_block_shifts();
                return;
            };
            let state = document.normal_block_shifts.as_mut().unwrap();
            state.set_ends(node, ends);
            if !state.can_defer(0.0) {
                document.abandon_normal_block_shifts();
                return;
            }
            state.mark_dirty(owner);
        } else if let Some(owner) = document.parent_of(node) {
            document
                .normal_block_shifts
                .as_mut()
                .unwrap()
                .mark_dirty(owner);
        }
    }
}

/// Reconstructs pre-move maxima from the already translated current geometry.
pub(super) fn extents(
    document: &crate::Document,
    parent: usize,
    moved: usize,
    flow_delta: f32,
    size_delta: f32,
    float_delta: f32,
    contains_floats: bool,
) -> Option<[(f32, f32); 3]> {
    let state = document.normal_block_shifts.as_ref()?;
    let (owner, index) = state.position(moved)?;
    if owner != parent {
        return None;
    }
    let len = state.len(parent)?;
    let prefix = state.range(parent, 0..index)?.0;
    let point = state.range(parent, index..index + 1)?.0;
    let suffix = state.range(parent, index + 1..len)?.0;
    if point.iter().all(|end| !end.is_finite()) {
        return None;
    }
    let delta = [
        if contains_floats {
            size_delta
        } else {
            flow_delta
        },
        size_delta,
        float_delta,
    ];
    let last = state
        .last_flow(parent)
        .and_then(|node| state.position(node).map(|(_, position)| (node, position)));
    let margin = if document.nodes[parent].layout.padding.bottom
        + document.nodes[parent].layout.border.bottom
        > 0.0
        || contains_floats
    {
        last.map_or(0.0, |(node, _)| document.nodes[node].layout.margin.bottom)
    } else {
        0.0
    };
    Some(std::array::from_fn(|channel| {
        if channel == 2 {
            return (
                prefix[channel]
                    .max(point[channel] - delta[channel])
                    .max(suffix[channel] - flow_delta),
                prefix[channel].max(point[channel]).max(suffix[channel]),
            );
        }
        let Some((_, last_index)) = last else {
            return (f32::NEG_INFINITY, f32::NEG_INFINITY);
        };
        let prior = state.range(parent, 0..index.min(last_index)).unwrap().0[channel];
        let current = if index < last_index {
            point[channel]
        } else {
            f32::NEG_INFINITY
        };
        let later = if index < last_index {
            state.range(parent, index + 1..last_index).unwrap().0[channel]
        } else {
            f32::NEG_INFINITY
        };
        let end = state.range(parent, last_index..last_index + 1).unwrap().0[channel];
        let last_delta = if last_index == index {
            delta[channel]
        } else if last_index > index {
            flow_delta
        } else {
            0.0
        };
        (
            prior
                .max(current - delta[channel])
                .max(later - flow_delta)
                .max((end - last_delta) + margin),
            prior.max(current).max(later).max(end + margin),
        )
    }))
}
