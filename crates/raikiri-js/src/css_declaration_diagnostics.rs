//! Optional per-thread logical work for CSS declaration validation and reads.

use std::cell::Cell;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub validation_calls: u64,
    pub raw_declarations: u64,
    pub linear_name_comparisons: u64,
    pub index_lookups: u64,
    pub index_removals: u64,
    pub index_insertions: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub max_cached_declarations: usize,
    pub max_cached_source_bytes: usize,
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
        total.validation_calls += work.validation_calls;
        total.raw_declarations += work.raw_declarations;
        total.linear_name_comparisons += work.linear_name_comparisons;
        total.index_lookups += work.index_lookups;
        total.index_removals += work.index_removals;
        total.index_insertions += work.index_insertions;
        total.cache_hits += work.cache_hits;
        total.cache_misses += work.cache_misses;
        total.max_cached_declarations = total
            .max_cached_declarations
            .max(work.max_cached_declarations);
        total.max_cached_source_bytes = total
            .max_cached_source_bytes
            .max(work.max_cached_source_bytes);
        ACTIVE.set(Some(total));
    }
}
