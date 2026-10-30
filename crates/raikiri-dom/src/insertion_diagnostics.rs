//! Optional logical work in insertion cycle checks.
#[cfg(any(test, feature = "insertion-diagnostics"))]
use std::cell::Cell;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub struct InsertionWork {
    pub calls: u64,
    pub parent_reads: u64,
    pub fast_paths: u64,
    pub dfs_searches: u64,
    pub dfs_visits: u64,
    pub visited_inserts: u64,
    pub scratch_growths: u64,
}

#[cfg(any(test, feature = "insertion-diagnostics"))]
thread_local! { static ACTIVE: Cell<Option<InsertionWork>> = const { Cell::new(None) }; }
#[cfg(any(test, feature = "insertion-diagnostics"))]
pub(crate) fn enabled() -> bool {
    ACTIVE.get().is_some()
}
#[cfg(any(test, feature = "insertion-diagnostics"))]
#[doc(hidden)]
pub fn begin_insertion_diagnostics() {
    ACTIVE.set(Some(InsertionWork::default()));
}
#[cfg(any(test, feature = "insertion-diagnostics"))]
#[doc(hidden)]
pub fn finish_insertion_diagnostics() -> InsertionWork {
    ACTIVE.replace(None).unwrap_or_default()
}

pub(crate) struct Meter<const COUNT: bool> {
    pub work: InsertionWork,
}
impl<const COUNT: bool> Meter<COUNT> {
    pub(crate) fn new() -> Self {
        Self {
            work: InsertionWork {
                calls: 1,
                ..Default::default()
            },
        }
    }
}
impl<const COUNT: bool> Drop for Meter<COUNT> {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "insertion-diagnostics"))]
        if COUNT && let Some(mut total) = ACTIVE.get() {
            total.calls = total.calls.saturating_add(self.work.calls);
            total.parent_reads = total.parent_reads.saturating_add(self.work.parent_reads);
            total.fast_paths = total.fast_paths.saturating_add(self.work.fast_paths);
            total.dfs_searches = total.dfs_searches.saturating_add(self.work.dfs_searches);
            total.dfs_visits = total.dfs_visits.saturating_add(self.work.dfs_visits);
            total.visited_inserts = total
                .visited_inserts
                .saturating_add(self.work.visited_inserts);
            total.scratch_growths = total
                .scratch_growths
                .saturating_add(self.work.scratch_growths);
            ACTIVE.set(Some(total));
        }
    }
}
