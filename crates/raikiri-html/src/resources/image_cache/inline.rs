//! Reservations for ownership kept outside the background cache entries.

use super::{Arc, DecodedImageCache, Mutex};
use std::sync::PoisonError;

/// One memo entry's reservation in its original resource cache and process.
/// This is not attached to caller-owned image clones and is never cloned.
pub(in crate::resources) struct InlineImageLease {
    cache: Arc<Mutex<DecodedImageCache>>,
    bytes: u64,
}

impl InlineImageLease {
    pub(in crate::resources) fn new(
        cache: &Arc<Mutex<DecodedImageCache>>,
        bytes: u64,
    ) -> Result<Self, (u64, u64)> {
        cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reserve_inline(bytes)?;
        Ok(Self {
            cache: Arc::clone(cache),
            bytes,
        })
    }
}

impl Drop for InlineImageLease {
    fn drop(&mut self) {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.inline_bytes = cache.inline_bytes.saturating_sub(self.bytes);
        cache.process_budget.release(self.bytes);
    }
}

impl DecodedImageCache {
    fn reserve_inline(&mut self, bytes: u64) -> Result<(), (u64, u64)> {
        let remaining = self.inline_remaining();
        if bytes > remaining {
            return Err((remaining, bytes));
        }
        if !self.process_budget.try_reserve(bytes) {
            return Err((remaining.min(self.process_budget.available_for(0)), bytes));
        }
        self.inline_bytes = self.inline_bytes.saturating_add(bytes);
        Ok(())
    }
}
