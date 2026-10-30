//! Optional per-thread measurements of table column reservations.

use std::cell::Cell;

/// Work performed while reserving a table's column slots.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReservationWork {
    /// Complete reservation passes, including repeated layout grid builds.
    pub passes: u64,
    /// Occupied slots skipped by the legacy linear search.
    pub occupied_slots: u64,
    /// Slots updated by the legacy reservation loop.
    pub slot_updates: u64,
    /// Tree nodes visited while searching for a free starting slot.
    pub search_nodes: u64,
    /// Tree nodes visited while updating a reservation range.
    pub update_nodes: u64,
    /// Tree nodes copied when capacity grows.
    pub copied_nodes: u64,
}

thread_local! {
    static ACTIVE: Cell<Option<ReservationWork>> = const { Cell::new(None) };
}

/// Start a fresh measurement on this thread.
pub fn begin() {
    ACTIVE.set(Some(ReservationWork::default()));
}

/// Finish the measurement, returning zero when none was active.
pub fn finish() -> ReservationWork {
    ACTIVE.replace(None).unwrap_or_default()
}

pub(crate) fn enabled() -> bool {
    ACTIVE.get().is_some()
}

#[inline]
pub(crate) fn record(update: impl FnOnce(&mut ReservationWork)) {
    if let Some(mut work) = ACTIVE.get() {
        update(&mut work);
        ACTIVE.set(Some(work));
    }
}
