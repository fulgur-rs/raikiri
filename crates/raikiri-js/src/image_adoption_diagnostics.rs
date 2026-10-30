//! Optional logical work counts for image membership checks during topology settlement.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub settlements: u64,
    pub active_checks: u64,
    pub inert_checks: u64,
}
#[cfg(any(test, feature = "image-adoption-diagnostics"))]
thread_local! { static ACTIVE: std::cell::Cell<Option<Work>> = const { std::cell::Cell::new(None) }; }
#[cfg(any(test, feature = "image-adoption-diagnostics"))]
pub fn begin() {
    ACTIVE.set(Some(Work::default()));
}
#[cfg(any(test, feature = "image-adoption-diagnostics"))]
pub fn finish() -> Work {
    ACTIVE.replace(None).unwrap_or_default()
}
pub(crate) fn enabled() -> bool {
    #[cfg(any(test, feature = "image-adoption-diagnostics"))]
    {
        ACTIVE.get().is_some()
    }
    #[cfg(not(any(test, feature = "image-adoption-diagnostics")))]
    {
        false
    }
}
pub(crate) fn record(_work: Work) {
    #[cfg(any(test, feature = "image-adoption-diagnostics"))]
    if let Some(mut total) = ACTIVE.get() {
        total.settlements += _work.settlements;
        total.active_checks += _work.active_checks;
        total.inert_checks += _work.inert_checks;
        ACTIVE.set(Some(total));
    }
}
