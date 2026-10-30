//! Optional work counts for live document base URL discovery.
#[cfg(any(test, feature = "base-url-diagnostics"))]
use std::cell::Cell;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub struct BaseUrlWork {
    pub searches: u64,
    pub node_visits: u64,
}
#[cfg(any(test, feature = "base-url-diagnostics"))]
thread_local! { static ACTIVE: Cell<Option<BaseUrlWork>> = const { Cell::new(None) }; }
#[cfg(any(test, feature = "base-url-diagnostics"))]
pub(crate) fn enabled() -> bool {
    ACTIVE.get().is_some()
}
#[cfg(any(test, feature = "base-url-diagnostics"))]
#[doc(hidden)]
pub fn begin_base_url_diagnostics() {
    ACTIVE.set(Some(BaseUrlWork::default()));
}
#[cfg(any(test, feature = "base-url-diagnostics"))]
#[doc(hidden)]
pub fn finish_base_url_diagnostics() -> BaseUrlWork {
    ACTIVE.replace(None).unwrap_or_default()
}
pub(crate) struct Meter<const COUNT: bool> {
    pub work: BaseUrlWork,
}
impl<const COUNT: bool> Meter<COUNT> {
    pub(crate) fn new() -> Self {
        Self {
            work: BaseUrlWork {
                searches: 1,
                node_visits: 0,
            },
        }
    }
}
impl<const COUNT: bool> Drop for Meter<COUNT> {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "base-url-diagnostics"))]
        if COUNT && let Some(mut total) = ACTIVE.get() {
            total.searches = total.searches.saturating_add(self.work.searches);
            total.node_visits = total.node_visits.saturating_add(self.work.node_visits);
            ACTIVE.set(Some(total));
        }
    }
}
