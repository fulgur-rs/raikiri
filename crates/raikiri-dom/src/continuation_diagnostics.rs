//! Optional per-thread measurements of paragraph ownership collection.

#[cfg(any(test, feature = "continuation-diagnostics"))]
use std::cell::Cell;

#[cfg(any(test, feature = "continuation-diagnostics"))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub struct ContinuationWork {
    pub text_line_visits: u64,
    pub glyph_owner_calls: u64,
}

#[cfg(any(test, feature = "continuation-diagnostics"))]
thread_local! {
    static ACTIVE: Cell<Option<ContinuationWork>> = const { Cell::new(None) };
}

#[cfg(any(test, feature = "continuation-diagnostics"))]
pub(crate) fn enabled() -> bool {
    ACTIVE.get().is_some()
}

#[cfg(any(test, feature = "continuation-diagnostics"))]
#[doc(hidden)]
pub fn begin_continuation_diagnostics() {
    ACTIVE.set(Some(ContinuationWork::default()));
}

#[cfg(any(test, feature = "continuation-diagnostics"))]
#[doc(hidden)]
pub fn finish_continuation_diagnostics() -> ContinuationWork {
    ACTIVE.replace(None).unwrap_or_default()
}

#[inline]
pub(crate) fn text_line_visit() {
    #[cfg(any(test, feature = "continuation-diagnostics"))]
    if let Some(mut work) = ACTIVE.get() {
        work.text_line_visits = work.text_line_visits.saturating_add(1);
        ACTIVE.set(Some(work));
    }
}

#[inline]
pub(crate) fn glyph_owner_call() {
    #[cfg(any(test, feature = "continuation-diagnostics"))]
    if let Some(mut work) = ACTIVE.get() {
        work.glyph_owner_calls = work.glyph_owner_calls.saturating_add(1);
        ACTIVE.set(Some(work));
    }
}
