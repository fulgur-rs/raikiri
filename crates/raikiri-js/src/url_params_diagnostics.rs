//! Optional logical counts of full URLSearchParams snapshots.
use std::cell::Cell;
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub snapshots: u64,
    pub pairs_cloned: u64,
}
thread_local! { static ACTIVE: Cell<Option<Work>> = const { Cell::new(None) }; }
pub fn begin() {
    ACTIVE.set(Some(Work::default()));
}
pub fn finish() -> Work {
    ACTIVE.replace(None).unwrap_or_default()
}
pub(crate) fn snapshot(pairs: usize) {
    if let Some(mut work) = ACTIVE.get() {
        work.snapshots = work.snapshots.saturating_add(1);
        work.pairs_cloned = work.pairs_cloned.saturating_add(pairs as u64);
        ACTIVE.set(Some(work));
    }
}
