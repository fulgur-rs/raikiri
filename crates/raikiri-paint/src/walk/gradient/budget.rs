//! Allocation limits for the conic stop pipeline during one paint call.

use std::cell::Cell;

/// Maximum entries in any one conic work buffer.
pub(in crate::walk) const MAX_VECTOR_ENTRIES: usize = 65_536;
const MAX_GRADIENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PAINT_BYTES: usize = 64 * 1024 * 1024;
const MAX_PAINT_ENTRIES: usize = 1_048_576;

#[derive(Clone, Copy, Default)]
struct Usage {
    bytes: usize,
    entries: usize,
}

/// Cumulative work for all conic backgrounds in one page paint call.
/// Reservations are not refunded: repeatedly allocating and discarding
/// buffers must not bypass the bound on allocation work.
#[derive(Default)]
pub(in crate::walk) struct ConicPaintBudget {
    used: Cell<Usage>,
}

impl ConicPaintBudget {
    pub(in crate::walk) fn for_gradient(&self) -> ConicBudget<'_> {
        ConicBudget {
            paint: self,
            bytes: Cell::new(0),
        }
    }
}

/// One gradient's work, also charged to its paint call.
pub(in crate::walk) struct ConicBudget<'a> {
    paint: &'a ConicPaintBudget,
    bytes: Cell<usize>,
}

impl ConicBudget<'_> {
    /// Reserve a known buffer or an owning copy before allocating it.
    pub(in crate::walk) fn charge<T>(&self, entries: usize) -> Option<()> {
        if entries > MAX_VECTOR_ENTRIES {
            return None;
        }
        let bytes = entries.checked_mul(size_of::<T>())?;
        let local_bytes = self.bytes.get().checked_add(bytes)?;
        let previous = self.paint.used.get();
        let used = Usage {
            bytes: previous.bytes.checked_add(bytes)?,
            entries: previous.entries.checked_add(entries)?,
        };
        if local_bytes > MAX_GRADIENT_BYTES
            || used.bytes > MAX_PAINT_BYTES
            || used.entries > MAX_PAINT_ENTRIES
        {
            return None;
        }
        self.bytes.set(local_bytes);
        self.paint.used.set(used);
        Some(())
    }

    pub(in crate::walk) fn vector<T>(&self, capacity: usize) -> Option<Vec<T>> {
        self.charge::<T>(capacity)?;
        let mut out = Vec::new();
        out.try_reserve_exact(capacity).ok()?;
        Some(out)
    }

    /// Grow explicitly before push, so Vec's automatic capacity doubling
    /// cannot allocate work that has not been reserved. Charge the full new
    /// allocation, including the old entries that reallocation copies.
    pub(in crate::walk) fn push<T>(&self, out: &mut Vec<T>, value: T) -> Option<()> {
        if out.len() == out.capacity() {
            let required = out.len().checked_add(1)?;
            if required > MAX_VECTOR_ENTRIES {
                return None;
            }
            let preferred = out
                .capacity()
                .max(2)
                .checked_mul(2)?
                .min(MAX_VECTOR_ENTRIES);
            if self.charge::<T>(preferred).is_none() {
                self.charge::<T>(required)?;
                out.try_reserve_exact(required - out.len()).ok()?;
            } else {
                out.try_reserve_exact(preferred - out.len()).ok()?;
            }
        }
        out.push(value);
        Some(())
    }
}

#[cfg(test)]
mod tests;
