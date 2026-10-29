//! Optional per-thread work counts; normal builds carry no counter state.

#[cfg(any(test, feature = "custom-property-diagnostics"))]
use std::cell::Cell;

/// Logical lookup and construction work, not elapsed time or allocator bytes.
#[cfg(any(test, feature = "custom-property-diagnostics"))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub struct CustomPropertyLookupWork {
    pub ancestor_visits: u64,
    pub index_visits: u64,
    pub created_nodes: u64,
    pub ownership_visits: u64,
}

#[cfg(any(test, feature = "custom-property-diagnostics"))]
thread_local! {
    static ACTIVE: Cell<Option<CustomPropertyLookupWork>> = const { Cell::new(None) };
}

#[cfg(any(test, feature = "custom-property-diagnostics"))]
pub(super) fn enabled() -> bool {
    ACTIVE.get().is_some()
}

/// Enable fresh counters for this thread; preparation can remain outside them.
#[cfg(any(test, feature = "custom-property-diagnostics"))]
#[doc(hidden)]
pub fn begin_custom_property_diagnostics() {
    ACTIVE.set(Some(CustomPropertyLookupWork::default()));
}

/// Disable and return this thread's counters, or zeros if already disabled.
#[cfg(any(test, feature = "custom-property-diagnostics"))]
#[doc(hidden)]
pub fn finish_custom_property_diagnostics() -> CustomPropertyLookupWork {
    ACTIVE.replace(None).unwrap_or_default()
}

#[inline]
pub(super) fn ancestor_visit() {
    #[cfg(any(test, feature = "custom-property-diagnostics"))]
    if let Some(mut work) = ACTIVE.get() {
        work.ancestor_visits = work.ancestor_visits.saturating_add(1);
        ACTIVE.set(Some(work));
    }
}
