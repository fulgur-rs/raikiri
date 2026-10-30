//! Optional logical work counts for class token membership reads.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub sets: u64,
    pub tokens: u64,
    pub dedup_comparisons: u64,
    pub owned_tokens: u64,
    pub attribute_bytes_cloned: u64,
    pub contains_comparisons: u64,
}
#[cfg(any(test, feature = "token-list-diagnostics"))]
thread_local! { static ACTIVE: std::cell::Cell<Option<Work>> = const { std::cell::Cell::new(None) }; }
#[cfg(any(test, feature = "token-list-diagnostics"))]
pub fn begin() {
    ACTIVE.set(Some(Work::default()));
}
#[cfg(any(test, feature = "token-list-diagnostics"))]
pub fn finish() -> Work {
    ACTIVE.replace(None).unwrap_or_default()
}
pub(crate) fn enabled() -> bool {
    #[cfg(any(test, feature = "token-list-diagnostics"))]
    {
        ACTIVE.get().is_some()
    }
    #[cfg(not(any(test, feature = "token-list-diagnostics")))]
    {
        false
    }
}
pub(crate) fn record(_work: Work) {
    #[cfg(any(test, feature = "token-list-diagnostics"))]
    if let Some(mut total) = ACTIVE.get() {
        total.sets += _work.sets;
        total.tokens += _work.tokens;
        total.dedup_comparisons += _work.dedup_comparisons;
        total.owned_tokens += _work.owned_tokens;
        total.attribute_bytes_cloned += _work.attribute_bytes_cloned;
        total.contains_comparisons += _work.contains_comparisons;
        ACTIVE.set(Some(total));
    }
}
