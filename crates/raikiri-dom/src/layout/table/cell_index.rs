use std::collections::BTreeSet;

use super::{CellPlacement, TableGrid};

#[derive(Clone, Copy)]
pub(super) struct Interval {
    pub(super) start: u16,
    pub(super) end: u16,
    pub(super) cell: usize,
}

/// Transient placement lookup shared by collapsed-border resolution and sizing.
/// Row spans occupy segment-tree ranges; each range stores disjoint column
/// intervals. Storage is O(rows + placements * log(rows)), independent of the
/// spanned grid area. Overlaps retain the first placement in grid order.
pub(super) struct CellIndex<'a> {
    leaf_count: usize,
    row_count: usize,
    column_count: u16,
    placements: &'a [CellPlacement],
    nodes: Vec<Vec<Interval>>,
    boundaries: std::cell::OnceCell<Option<super::boundaries::BoundarySegments>>,
}

impl<'a> CellIndex<'a> {
    pub(super) fn new(grid: &'a TableGrid) -> Self {
        let row_count = grid.rows.len();
        let leaf_count = row_count.max(1).next_power_of_two();
        let mut index = Self {
            leaf_count,
            row_count,
            column_count: grid.n_cols,
            placements: &grid.cells,
            boundaries: std::cell::OnceCell::new(),
            nodes: if row_count == 0 || grid.n_cols == 0 {
                Vec::new()
            } else {
                vec![Vec::new(); 2 * leaf_count]
            },
        };
        if index.nodes.is_empty() {
            return index;
        }
        for (cell, placement) in grid.cells.iter().enumerate() {
            let start = usize::from(placement.col_start);
            let end = (start + usize::from(placement.col_span)).min(usize::from(grid.n_cols));
            let mut row_start = usize::from(placement.row);
            let mut row_end = (row_start + usize::from(placement.row_span)).min(row_count);
            if start >= end || row_start >= row_end {
                continue;
            }
            let interval = Interval {
                start: start as u16,
                end: end as u16,
                cell,
            };
            row_start += leaf_count;
            row_end += leaf_count;
            while row_start < row_end {
                if row_start % 2 == 1 {
                    index.nodes[row_start].push(interval);
                    row_start += 1;
                }
                if row_end % 2 == 1 {
                    row_end -= 1;
                    index.nodes[row_end].push(interval);
                }
                row_start /= 2;
                row_end /= 2;
            }
        }
        for intervals in &mut index.nodes {
            make_disjoint(intervals);
        }
        index
    }

    pub(super) fn at(&self, row: u16, col: u16) -> Option<usize> {
        if usize::from(row) >= self.row_count || col >= self.column_count {
            return None;
        }
        let mut node = self.leaf_count + usize::from(row);
        let mut first = None;
        let mut placements = self.placements.iter().enumerate();
        while node > 0 {
            let intervals = &self.nodes[node];
            // Race a linear prefix against indexed work. Each node advances
            // at most its visit plus a binary-search comparison bound, so an
            // early placement stays O(its position) without a fixed cutoff.
            let budget = 1 + intervals
                .len()
                .checked_ilog2()
                .map_or(0, |bits| bits as usize + 2);
            for _ in 0..budget {
                let (cell, placement) = placements.next()?;
                lookup_step();
                if usize::from(row) >= usize::from(placement.row)
                    && usize::from(row)
                        < usize::from(placement.row) + usize::from(placement.row_span)
                    && usize::from(col) >= usize::from(placement.col_start)
                    && usize::from(col)
                        < usize::from(placement.col_start) + usize::from(placement.col_span)
                {
                    return Some(cell);
                }
            }
            if placements.len() == 0 {
                return None;
            }
            lookup_step();
            let end = intervals.partition_point(|interval| {
                lookup_step();
                interval.start <= col
            });
            if end > 0 {
                let interval = intervals[end - 1];
                if col < interval.end {
                    first =
                        Some(first.map_or(interval.cell, |cell: usize| cell.min(interval.cell)));
                }
            }
            node /= 2;
        }
        first
    }

    pub(super) fn boundary_segments(
        &self,
        grid: &TableGrid,
    ) -> Option<&super::boundaries::BoundarySegments> {
        self.boundaries
            .get_or_init(|| super::boundaries::BoundarySegments::new(grid, self))
            .as_ref()
    }

    #[cfg(test)]
    pub(super) fn disable_sparse(&self) {
        assert!(self.boundaries.set(None).is_ok());
    }

    /// Merge every tree level by placement ordinal, preserving holes.
    pub(super) fn row_runs(&self, row: usize, limit: usize) -> Option<Vec<Interval>> {
        if row >= self.row_count || self.nodes.is_empty() {
            return Some(Vec::new());
        }
        let mut runs = Vec::new();
        let mut node = self.leaf_count + row;
        while node > 0 {
            let intervals = &self.nodes[node];
            super::boundaries::interval_visits(intervals.len());
            if !intervals.is_empty() {
                if runs.is_empty() {
                    if intervals.len() > limit {
                        return None;
                    }
                    runs.try_reserve_exact(intervals.len()).ok()?;
                    runs.extend_from_slice(intervals);
                } else {
                    runs = merge_runs(&runs, intervals, limit)?;
                }
            }
            node /= 2;
        }
        Some(runs)
    }

    #[cfg(test)]
    pub(super) fn storage_entries(&self) -> usize {
        self.nodes.iter().map(Vec::len).sum()
    }
}

/// Resolve column overlaps within one row range without expanding any span.
fn make_disjoint(intervals: &mut Vec<Interval>) {
    if intervals.len() < 2 {
        return;
    }
    intervals.sort_unstable_by_key(|interval| interval.start);
    if intervals
        .windows(2)
        .all(|pair| pair[0].end <= pair[1].start)
    {
        return;
    }
    let mut events = Vec::with_capacity(2 * intervals.len());
    for interval in intervals.iter() {
        events.push((interval.start, interval.cell, true));
        events.push((interval.end, interval.cell, false));
    }
    events.sort_unstable_by_key(|event| event.0);
    intervals.clear();
    let mut active = BTreeSet::new();
    let mut previous = 0;
    let mut offset = 0;
    while offset < events.len() {
        let column = events[offset].0;
        if previous < column
            && let Some(&cell) = active.first()
        {
            if let Some(last) = intervals.last_mut()
                && last.cell == cell
                && last.end == previous
            {
                last.end = column;
            } else {
                intervals.push(Interval {
                    start: previous,
                    end: column,
                    cell,
                });
            }
        }
        while offset < events.len() && events[offset].0 == column {
            let (_, cell, enter) = events[offset];
            if enter {
                active.insert(cell);
            } else {
                active.remove(&cell);
            }
            offset += 1;
        }
        previous = column;
    }
}

#[inline]
fn lookup_step() {
    #[cfg(test)]
    crate::layout::test_support::TABLE_CELL_LOOKUP_STEPS.with(|steps| steps.set(steps.get() + 1));
}

/// Overlay two disjoint sorted lists without expanding their coordinates.
fn merge_runs(a: &[Interval], b: &[Interval], limit: usize) -> Option<Vec<Interval>> {
    let mut result: Vec<Interval> = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut x = a.first()?.start.min(b.first()?.start);
    loop {
        while i < a.len() && a[i].end <= x {
            i += 1;
        }
        while j < b.len() && b[j].end <= x {
            j += 1;
        }
        super::boundaries::merge_visits(1);
        let next_edge = |list: &[Interval], offset: usize| {
            list.get(offset)
                .map(|run| if run.start > x { run.start } else { run.end })
        };
        let end = match (next_edge(a, i), next_edge(b, j)) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => break,
        };
        let cell_at = |list: &[Interval], offset: usize| {
            list.get(offset)
                .filter(|run| run.start <= x)
                .map(|run| run.cell)
        };
        let cell = match (cell_at(a, i), cell_at(b, j)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if let Some(cell) = cell {
            if let Some(last) = result.last_mut()
                && last.cell == cell
                && last.end == x
            {
                last.end = end;
            } else {
                super::boundaries::bounded_push(
                    &mut result,
                    Interval {
                        start: x,
                        end,
                        cell,
                    },
                    limit,
                )?;
            }
        }
        x = end;
    }
    Some(result)
}
