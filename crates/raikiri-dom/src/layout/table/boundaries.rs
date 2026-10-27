use super::{CellIndex, TableGrid};

// Both segment vectors together retain at most 4 MiB. Three simultaneous
// interval vectors use at most 1.5 MiB; row events use less than 600 KiB.
// Capacity growth is explicit, so transient vector payload stays below 8 MiB.
const SEGMENT_BYTES_PER_AXIS: usize = 2 * 1024 * 1024;
const RUN_LIMIT: usize = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Segment {
    pub(super) boundary: usize,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) leading: usize,
    pub(super) trailing: usize,
}

pub(super) struct BoundarySegments {
    pub(super) vertical: Vec<Segment>,
    pub(super) horizontal: Vec<Segment>,
}

impl BoundarySegments {
    pub(super) fn new(grid: &TableGrid, index: &CellIndex) -> Option<Self> {
        record(4, 1);
        let proportional = grid
            .cells
            .len()
            .saturating_add(grid.rows.len())
            .saturating_add(usize::from(grid.n_cols))
            .saturating_mul(4)
            .saturating_add(64);
        let limit = proportional.min(SEGMENT_BYTES_PER_AXIS / std::mem::size_of::<Segment>());
        Self::new_bounded(grid, index, limit)
    }

    pub(super) fn new_bounded(grid: &TableGrid, index: &CellIndex, limit: usize) -> Option<Self> {
        // Old coordinate casts wrap beyond this domain; preserve that path.
        if grid.rows.len() > usize::from(u16::MAX) + 1 {
            return None;
        }
        let n = usize::from(grid.n_cols);
        let m = grid.rows.len();
        let mut result = Self {
            vertical: Vec::new(),
            horizontal: Vec::new(),
        };
        if n == 0 || m == 0 {
            return Some(result);
        }
        let mut events = vec![false; m + 1];
        events[0] = true;
        events[m] = true;
        for cell in &grid.cells {
            let start = usize::from(cell.row).min(m);
            let end = (usize::from(cell.row) + usize::from(cell.row_span)).min(m);
            events[start] = true;
            events[end] = true;
        }
        let mut rows = Vec::new();
        rows.try_reserve_exact(events.iter().filter(|&&event| event).count())
            .ok()?;
        rows.extend(
            events
                .iter()
                .enumerate()
                .filter_map(|(row, &event)| event.then_some(row)),
        );
        drop(events);
        let mut previous = Vec::new();
        let run_limit = RUN_LIMIT.min(limit);
        for pair in rows.windows(2) {
            let (row, end) = (pair[0], pair[1]);
            let runs = index.row_runs(row, run_limit)?;
            if let Some(first) = runs.first()
                && first.start == 0
            {
                bounded_push(
                    &mut result.vertical,
                    Segment {
                        boundary: 0,
                        start: row,
                        end,
                        leading: first.cell,
                        trailing: first.cell,
                    },
                    limit,
                )?;
                emitted_segments(1);
            }
            for neighbours in runs.windows(2) {
                let (left, right) = (neighbours[0], neighbours[1]);
                if left.end == right.start {
                    bounded_push(
                        &mut result.vertical,
                        Segment {
                            boundary: usize::from(left.end),
                            start: row,
                            end,
                            leading: left.cell,
                            trailing: right.cell,
                        },
                        limit,
                    )?;
                    emitted_segments(1);
                }
            }
            if let Some(last) = runs.last()
                && usize::from(last.end) == n
            {
                bounded_push(
                    &mut result.vertical,
                    Segment {
                        boundary: n,
                        start: row,
                        end,
                        leading: last.cell,
                        trailing: last.cell,
                    },
                    limit,
                )?;
                emitted_segments(1);
            }
            let (mut a, mut b) = (0, 0);
            while a < previous.len() && b < runs.len() {
                let above: super::cell_index::Interval = previous[a];
                let below = runs[b];
                let start = above.start.max(below.start);
                let end = above.end.min(below.end);
                if start < end {
                    bounded_push(
                        &mut result.horizontal,
                        Segment {
                            boundary: row,
                            start: usize::from(start),
                            end: usize::from(end),
                            leading: above.cell,
                            trailing: below.cell,
                        },
                        limit,
                    )?;
                    emitted_segments(1);
                }
                if above.end <= below.end {
                    a += 1;
                }
                if below.end <= above.end {
                    b += 1;
                }
            }
            previous = runs;
        }
        result
            .vertical
            .sort_unstable_by_key(|segment| (segment.boundary, segment.start));
        retained_bytes(
            (result.vertical.capacity() + result.horizontal.capacity())
                * std::mem::size_of::<Segment>(),
        );
        Some(result)
    }
}

pub(super) fn bounded_push<T>(values: &mut Vec<T>, value: T, limit: usize) -> Option<()> {
    if values.len() == limit {
        return None;
    }
    if values.len() == values.capacity() {
        let capacity = values.capacity().saturating_mul(2).max(4).min(limit);
        values.try_reserve_exact(capacity - values.len()).ok()?;
    }
    values.push(value);
    Some(())
}

#[cfg(test)]
thread_local! {
    pub(super) static WORK: std::cell::Cell<[usize; 5]> = const { std::cell::Cell::new([0; 5]) };
}

pub(super) fn interval_visits(count: usize) {
    record(0, count);
}
pub(super) fn merge_visits(count: usize) {
    record(1, count);
}
fn emitted_segments(count: usize) {
    record(2, count);
}
fn retained_bytes(_count: usize) {
    #[cfg(test)]
    WORK.with(|work| {
        let mut values = work.get();
        values[3] = values[3].max(_count);
        work.set(values);
    });
}
fn record(_field: usize, _count: usize) {
    #[cfg(test)]
    WORK.with(|work| {
        let mut values = work.get();
        values[_field] += _count;
        work.set(values);
    });
}

#[cfg(test)]
impl Drop for BoundarySegments {
    fn drop(&mut self) {
        retained_bytes(
            (self.vertical.capacity() + self.horizontal.capacity())
                * std::mem::size_of::<Segment>(),
        );
    }
}
