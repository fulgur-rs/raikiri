//! Optional operation counts for the external accounting benchmark.

#[derive(Default)]
pub(super) struct AccountingWork {
    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
    enabled: std::cell::Cell<bool>,
    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
    count: std::cell::Cell<u64>,
}

impl AccountingWork {
    #[inline]
    pub(super) fn record(&self, _units: u64) {
        #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
        if self.enabled.get() {
            self.count.set(self.count.get().saturating_add(_units));
        }
    }

    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
    pub(super) fn begin(&self, enabled: bool) {
        self.count.set(0);
        self.enabled.set(enabled);
    }

    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
    pub(super) fn finish(&self) {
        self.enabled.set(false);
    }

    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
    pub(super) fn value(&self) -> u64 {
        self.count.get()
    }
}
