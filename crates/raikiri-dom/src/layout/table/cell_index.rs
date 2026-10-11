use std::collections::BTreeSet;
use std::ops::Range;

use super::TableGrid;

#[derive(Clone, Copy)]
struct Interval {
    start: u16,
    end: u16,
    cell: usize,
}

struct CellRegion {
    rows: Range<usize>,
    columns: Range<u16>,
}

/// Transient placement lookup shared by collapsed-border resolution and sizing.
/// Row spans occupy segment-tree ranges; each range stores disjoint column
/// intervals. Storage is O(rows + placements * log(rows)), independent of the
/// spanned grid area. Overlaps retain the first placement in grid order.
pub(super) struct CellIndex {
    leaf_count: usize,
    row_count: usize,
    column_count: u16,
    first_cell: Option<CellRegion>,
    single_cell: bool,
    nodes: Vec<Vec<Interval>>,
}

impl CellIndex {
    pub(super) fn new(grid: &TableGrid) -> Self {
        let row_count = grid.rows.len();
        let leaf_count = row_count.max(1).next_power_of_two();
        let mut index = Self {
            leaf_count,
            row_count,
            column_count: grid.n_cols,
            first_cell: None,
            single_cell: grid.cells.len() <= 1,
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
            if cell == 0 {
                index.first_cell = Some(CellRegion {
                    rows: row_start..row_end,
                    columns: start as u16..end as u16,
                });
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
        // Nothing can precede placement zero. Keep its covered positions
        // constant-time even when one cell spans a large, otherwise empty grid.
        if let Some(region) = &self.first_cell {
            lookup_step();
            if region.rows.contains(&usize::from(row)) && region.columns.contains(&col) {
                return Some(0);
            }
        }
        if self.single_cell {
            return None;
        }
        let mut node = self.leaf_count + usize::from(row);
        let mut first = None;
        while node > 0 {
            lookup_step();
            let intervals = &self.nodes[node];
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
