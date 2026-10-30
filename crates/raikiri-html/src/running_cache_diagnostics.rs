//! Optional logical work per running-layout cache operation.

#[cfg(any(test, feature = "running-cache-diagnostics"))]
use std::cell::Cell;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub struct RunningCacheWork {
    pub requests: u64,
    pub lookups: u64,
    pub width_comparisons: u64,
    pub make_calls: u64,
    pub created_layouts: u64,
    pub admitted_entries: u64,
    pub budget_denials: u64,
    pub max_reserved_entries: usize,
}

#[cfg(any(test, feature = "running-cache-diagnostics"))]
thread_local! {
    static ACTIVE: Cell<Option<RunningCacheWork>> = const { Cell::new(None) };
}

#[cfg(any(test, feature = "running-cache-diagnostics"))]
pub(crate) fn enabled() -> bool {
    ACTIVE.get().is_some()
}

#[cfg(any(test, feature = "running-cache-diagnostics"))]
#[doc(hidden)]
pub fn begin_running_cache_diagnostics() {
    ACTIVE.set(Some(RunningCacheWork::default()));
}

#[cfg(any(test, feature = "running-cache-diagnostics"))]
#[doc(hidden)]
pub fn finish_running_cache_diagnostics() -> RunningCacheWork {
    ACTIVE.replace(None).unwrap_or_default()
}

pub(crate) struct Meter<const COUNT: bool> {
    pub work: RunningCacheWork,
}

impl<const COUNT: bool> Meter<COUNT> {
    pub(crate) fn new() -> Self {
        Self {
            work: RunningCacheWork {
                requests: 1,
                ..Default::default()
            },
        }
    }
}

impl<const COUNT: bool> Drop for Meter<COUNT> {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "running-cache-diagnostics"))]
        if COUNT && let Some(mut total) = ACTIVE.get() {
            let work = self.work;
            total.requests = total.requests.saturating_add(work.requests);
            total.lookups = total.lookups.saturating_add(work.lookups);
            total.width_comparisons = total
                .width_comparisons
                .saturating_add(work.width_comparisons);
            total.make_calls = total.make_calls.saturating_add(work.make_calls);
            total.created_layouts = total.created_layouts.saturating_add(work.created_layouts);
            total.admitted_entries = total.admitted_entries.saturating_add(work.admitted_entries);
            total.budget_denials = total.budget_denials.saturating_add(work.budget_denials);
            total.max_reserved_entries = total.max_reserved_entries.max(work.max_reserved_entries);
            ACTIVE.set(Some(total));
        }
    }
}
