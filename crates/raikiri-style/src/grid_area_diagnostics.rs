//! Optional per-thread logical work for named grid-area bounds construction.

use std::cell::Cell;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub linear_comparisons: u64,
    pub indexed_lookups: u64,
    pub indexed_insertions: u64,
    pub rectangle_cells: u64,
}

thread_local! {
    static ACTIVE: Cell<Option<Work>> = const { Cell::new(None) };
}

pub fn begin() {
    ACTIVE.set(Some(Work::default()));
}

pub fn finish() -> Work {
    ACTIVE.replace(None).unwrap_or_default()
}

pub(crate) fn enabled() -> bool {
    ACTIVE.get().is_some()
}

pub(crate) fn record(work: Work) {
    if let Some(mut total) = ACTIVE.get() {
        total.linear_comparisons += work.linear_comparisons;
        total.indexed_lookups += work.indexed_lookups;
        total.indexed_insertions += work.indexed_insertions;
        total.rectangle_cells += work.rectangle_cells;
        ACTIVE.set(Some(total));
    }
}
