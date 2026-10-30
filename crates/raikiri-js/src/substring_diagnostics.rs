//! Optional logical work counts for CharacterData substring reads.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub calls: u64,
    pub units_visited: u64,
    pub temporary_units: u64,
    pub result_units: u64,
}
#[cfg(any(test, feature = "substring-diagnostics"))]
thread_local! { static ACTIVE: std::cell::Cell<Option<Work>> = const { std::cell::Cell::new(None) }; }
#[cfg(any(test, feature = "substring-diagnostics"))]
pub fn begin() {
    ACTIVE.set(Some(Work::default()));
}
#[cfg(any(test, feature = "substring-diagnostics"))]
pub fn finish() -> Work {
    ACTIVE.replace(None).unwrap_or_default()
}
pub(crate) fn enabled() -> bool {
    #[cfg(any(test, feature = "substring-diagnostics"))]
    {
        ACTIVE.get().is_some()
    }
    #[cfg(not(any(test, feature = "substring-diagnostics")))]
    {
        false
    }
}
pub(crate) fn record(_work: Work) {
    #[cfg(any(test, feature = "substring-diagnostics"))]
    if let Some(mut total) = ACTIVE.get() {
        total.calls += _work.calls;
        total.units_visited += _work.units_visited;
        total.temporary_units += _work.temporary_units;
        total.result_units += _work.result_units;
        ACTIVE.set(Some(total));
    }
}
