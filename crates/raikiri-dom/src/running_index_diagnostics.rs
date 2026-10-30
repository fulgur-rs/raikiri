//! Optional logical work counts for locating rendered neighbors.

#[cfg(any(test, feature = "running-index-diagnostics"))]
use std::cell::Cell;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub struct RunningIndexWork {
    pub next_candidate_visits: u64,
    pub previous_candidate_visits: u64,
    pub own_candidate_visits: u64,
    pub prepared_nodes: u64,
    pub prepared_slots: u64,
    pub indexed_probes: u64,
    pub index_builds: u64,
    pub scratch_bytes: usize,
}

#[cfg(any(test, feature = "running-index-diagnostics"))]
thread_local! {
    static ACTIVE: Cell<Option<RunningIndexWork>> = const { Cell::new(None) };
}

#[cfg(any(test, feature = "running-index-diagnostics"))]
pub(crate) fn enabled() -> bool {
    ACTIVE.get().is_some()
}

#[cfg(any(test, feature = "running-index-diagnostics"))]
#[doc(hidden)]
pub fn begin_running_index_diagnostics() {
    ACTIVE.set(Some(RunningIndexWork::default()));
}

#[cfg(any(test, feature = "running-index-diagnostics"))]
#[doc(hidden)]
pub fn finish_running_index_diagnostics() -> RunningIndexWork {
    ACTIVE.replace(None).unwrap_or_default()
}

#[cfg(any(test, feature = "running-index-diagnostics"))]
pub(crate) fn record(work: RunningIndexWork) {
    if let Some(mut total) = ACTIVE.get() {
        total.next_candidate_visits = total
            .next_candidate_visits
            .saturating_add(work.next_candidate_visits);
        total.previous_candidate_visits = total
            .previous_candidate_visits
            .saturating_add(work.previous_candidate_visits);
        total.own_candidate_visits = total
            .own_candidate_visits
            .saturating_add(work.own_candidate_visits);
        total.prepared_nodes = total.prepared_nodes.saturating_add(work.prepared_nodes);
        total.prepared_slots = total.prepared_slots.saturating_add(work.prepared_slots);
        total.indexed_probes = total.indexed_probes.saturating_add(work.indexed_probes);
        total.index_builds = total.index_builds.saturating_add(work.index_builds);
        total.scratch_bytes = total.scratch_bytes.max(work.scratch_bytes);
        ACTIVE.set(Some(total));
    }
}
