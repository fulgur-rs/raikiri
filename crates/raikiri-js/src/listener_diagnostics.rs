//! Optional logical work counts for listener snapshots, liveness lookup, and compaction.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub invocations: u64,
    pub snapshot_candidates: u64,
    pub snapshot_entries: u64,
    pub kind_bytes_cloned: u64,
    pub lookup_checks: u64,
    pub liveness_checks: u64,
    pub compactions: u64,
    pub compaction_scans: u64,
}
#[cfg(any(test, feature = "listener-diagnostics"))]
thread_local! { static ACTIVE: std::cell::Cell<Option<Work>> = const { std::cell::Cell::new(None) }; }
#[cfg(any(test, feature = "listener-diagnostics"))]
pub fn begin() {
    ACTIVE.set(Some(Work::default()));
}
#[cfg(any(test, feature = "listener-diagnostics"))]
pub fn finish() -> Work {
    ACTIVE.replace(None).unwrap_or_default()
}
pub(crate) fn enabled() -> bool {
    #[cfg(any(test, feature = "listener-diagnostics"))]
    {
        ACTIVE.get().is_some()
    }
    #[cfg(not(any(test, feature = "listener-diagnostics")))]
    {
        false
    }
}
pub(crate) fn record(_work: Work) {
    #[cfg(any(test, feature = "listener-diagnostics"))]
    if let Some(mut total) = ACTIVE.get() {
        total.invocations += _work.invocations;
        total.snapshot_candidates += _work.snapshot_candidates;
        total.snapshot_entries += _work.snapshot_entries;
        total.kind_bytes_cloned += _work.kind_bytes_cloned;
        total.lookup_checks += _work.lookup_checks;
        total.liveness_checks += _work.liveness_checks;
        total.compactions += _work.compactions;
        total.compaction_scans += _work.compaction_scans;
        ACTIVE.set(Some(total));
    }
}

pub(crate) fn compaction(_entries: usize) {
    #[cfg(any(test, feature = "listener-diagnostics"))]
    if let Some(mut total) = ACTIVE.get() {
        total.compactions += 1;
        total.compaction_scans += _entries as u64;
        ACTIVE.set(Some(total));
    }
}
#[derive(Debug, Clone, Copy)]
pub struct StorageSizes {
    pub node_state: usize,
    pub node_record: usize,
    pub list_field: usize,
    pub list_data: usize,
    pub entry_data: usize,
}
/// Rust layout bytes, excluding GC headers and transitive heap allocations.
#[cfg(any(test, feature = "listener-diagnostics"))]
pub fn storage_sizes() -> StorageSizes {
    crate::runtime::listener_storage_sizes()
}
