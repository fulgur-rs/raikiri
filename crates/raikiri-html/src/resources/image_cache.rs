//! Decoded background image caches and the process-wide decode budget
//! shared by every resource set.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use raikiri_svg::SvgDocument;
use raikiri_traits::{DecodedImage, ImageRasterSize, RenderWarning};
use url::Url;

use super::DEFAULT_MAX_DECODED_IMAGE_CACHE_BYTES;

/// Maximum CSS background image decodes that may run at once, process-wide.
///
/// Independent [`RenderResources`] instances share this semaphore, so decoder
/// concurrency stays bounded no matter how many resource contexts exist.
/// Unrelated images proceed concurrently up to this bound instead of sharing
/// one exclusive mutex; only work beyond the bound waits.
pub(super) const PROCESS_MAX_CONCURRENT_IMAGE_DECODES: usize = 4;
/// Maximum decoded CSS background raster bytes retained, process-wide.
///
/// Each resource set keeps its own 128 MiB cache, but the sum across all live
/// sets is additionally capped here so independent contexts cannot grow the
/// process peak without bound. Inserts that would exceed the process cap fail
/// with the same per-image limit warning the per-set cap produces.
pub(super) const PROCESS_MAX_RETAINED_DECODED_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
pub(super) enum CachedBackgroundSource {
    Raster(Arc<DecodedImage>),
    Svg(Arc<SvgDocument>),
}

/// Rasters kept per background image, one per paint size, so that an SVG
/// painted at a few sizes in turn is not rasterized again at every switch.
const MAX_RASTER_SIZES: usize = 4;

#[derive(Clone)]
pub(super) struct CachedBackgroundImage {
    pub(super) source: CachedBackgroundSource,
    /// Rasters by paint size, oldest first.
    pub(super) rasters: Vec<(ImageRasterSize, Arc<DecodedImage>)>,
}

impl CachedBackgroundImage {
    pub(super) fn raster(&self, size: ImageRasterSize) -> Option<Arc<DecodedImage>> {
        self.rasters
            .iter()
            .find(|(cached, _)| *cached == size)
            .map(|(_, image)| image.clone())
    }
}

fn image_bytes(image: &DecodedImage) -> u64 {
    u64::try_from(image.rgba.len()).unwrap_or(u64::MAX)
}

/// Process-wide budget shared by every [`RenderResources`] for CSS background decoding.
///
/// The per-set 128 MiB cache bounds one security context; this object bounds
/// their sum plus concurrent decoder slots process-wide. Reconfigured
/// [`RenderResources`] clones keep sharing the same budget object while their
/// caches, provider budgets, and in-flight maps are replaced, so cache and
/// policy isolation holds while decoder concurrency and retained peak stay
/// bounded across independent contexts.
#[derive(Debug)]
pub(crate) struct ProcessImageBudget {
    retained: Mutex<u64>,
    max_retained: u64,
    permits: Mutex<usize>,
    cvar: Condvar,
    max_permits: usize,
}

impl ProcessImageBudget {
    pub(super) fn global() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<ProcessImageBudget>> = OnceLock::new();
        GLOBAL
            .get_or_init(|| {
                Arc::new(Self {
                    retained: Mutex::new(0),
                    max_retained: PROCESS_MAX_RETAINED_DECODED_BYTES,
                    permits: Mutex::new(PROCESS_MAX_CONCURRENT_IMAGE_DECODES),
                    cvar: Condvar::new(),
                    max_permits: PROCESS_MAX_CONCURRENT_IMAGE_DECODES,
                })
            })
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn new(max_retained: u64, max_concurrent: usize) -> Self {
        Self {
            retained: Mutex::new(0),
            max_retained,
            permits: Mutex::new(max_concurrent),
            cvar: Condvar::new(),
            max_permits: max_concurrent.max(1),
        }
    }

    #[cfg(test)]
    pub(crate) fn max_retained(&self) -> u64 {
        self.max_retained
    }

    #[cfg(test)]
    pub(crate) fn retained(&self) -> u64 {
        *self
            .retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn available_for(&self, prior: u64) -> u64 {
        let retained = *self
            .retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.max_retained
            .saturating_sub(retained.saturating_sub(prior))
    }

    fn try_reserve(&self, delta: u64) -> bool {
        let mut retained = self
            .retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = retained.saturating_add(delta);
        if next > self.max_retained {
            return false;
        }
        *retained = next;
        true
    }

    fn release(&self, amount: u64) {
        if amount == 0 {
            return;
        }
        let mut retained = self
            .retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *retained = retained.saturating_sub(amount);
    }

    pub(super) fn acquire(self: &Arc<Self>) -> ProcessDecodeGuard {
        let mut permits = self
            .permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *permits == 0 {
            permits = self
                .cvar
                .wait(permits)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *permits -= 1;
        ProcessDecodeGuard {
            budget: Arc::clone(self),
        }
    }

    #[cfg(test)]
    pub(super) fn try_acquire(self: &Arc<Self>) -> Option<ProcessDecodeGuard> {
        let mut permits = self
            .permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *permits == 0 {
            return None;
        }
        *permits -= 1;
        Some(ProcessDecodeGuard {
            budget: Arc::clone(self),
        })
    }
}

/// Permit holding one process-wide decoder slot.
///
/// The slot is held for the whole synchronous decode, including the wall-clock
/// window a timed-out or aborted decode still occupies. Releasing happens only
/// when the decoding thread returns, so timed-out or aborted work retains its
/// budget until it stops.
pub(crate) struct ProcessDecodeGuard {
    budget: Arc<ProcessImageBudget>,
}

impl Drop for ProcessDecodeGuard {
    fn drop(&mut self) {
        let mut permits = self
            .budget
            .permits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *permits = (*permits + 1).min(self.budget.max_permits);
        self.budget.cvar.notify_one();
    }
}

struct InflightState {
    done: bool,
    warning: Option<RenderWarning>,
}

pub(super) struct BackgroundInflight {
    state: Mutex<InflightState>,
    cvar: Condvar,
}

impl BackgroundInflight {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(InflightState {
                done: false,
                warning: None,
            }),
            cvar: Condvar::new(),
        }
    }

    pub(super) fn wait_for_done(&self) -> Option<RenderWarning> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !state.done {
            state = self
                .cvar
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.warning.clone()
    }

    pub(super) fn finish(&self, warning: Option<RenderWarning>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.warning = warning;
        state.done = true;
        self.cvar.notify_all();
    }
}

pub(super) struct DecodedImageCache {
    pub(super) entries: HashMap<Url, CachedBackgroundImage>,
    pub(super) decoded_bytes: u64,
    pub(super) max_bytes: u64,
    pub(super) process_budget: Arc<ProcessImageBudget>,
}

impl DecodedImageCache {
    pub(super) fn new(process_budget: Arc<ProcessImageBudget>) -> Self {
        Self {
            entries: HashMap::new(),
            decoded_bytes: 0,
            max_bytes: DEFAULT_MAX_DECODED_IMAGE_CACHE_BYTES,
            process_budget,
        }
    }

    pub(super) fn prior_bytes(&self, url: &Url) -> u64 {
        self.entries.get(url).map(Self::raster_bytes).unwrap_or(0)
    }
}

impl Drop for DecodedImageCache {
    fn drop(&mut self) {
        self.process_budget.release(self.decoded_bytes);
    }
}

impl DecodedImageCache {
    fn raster_bytes(entry: &CachedBackgroundImage) -> u64 {
        if entry.rasters.is_empty() {
            return match &entry.source {
                CachedBackgroundSource::Raster(image) => image_bytes(image),
                CachedBackgroundSource::Svg(_) => 0,
            };
        }
        entry.rasters.iter().fold(0, |sum: u64, (_, image)| {
            sum.saturating_add(image_bytes(image))
        })
    }

    pub(super) fn remaining_for(&self, url: &Url) -> u64 {
        let existing = self.prior_bytes(url);
        self.max_bytes
            .saturating_sub(self.decoded_bytes.saturating_sub(existing))
    }

    pub(super) fn insert_source(&mut self, url: Url, source: CachedBackgroundSource) {
        let previous = self.entries.remove(&url);
        if let Some(previous) = previous {
            let prior = Self::raster_bytes(&previous);
            if prior > 0 {
                self.decoded_bytes = self.decoded_bytes.saturating_sub(prior);
                self.process_budget.release(prior);
            }
        }
        self.entries.insert(
            url,
            CachedBackgroundImage {
                source,
                rasters: Vec::new(),
            },
        );
    }

    /// Cache `image` as the raster of `url` at `size`, next to the rasters
    /// of other sizes. The oldest sizes make room when the size count or a
    /// byte budget would be exceeded; the error reports the budget left for
    /// `image` alone when even that does not fit.
    pub(super) fn insert_raster(
        &mut self,
        url: Url,
        source: CachedBackgroundSource,
        size: ImageRasterSize,
        image: Arc<DecodedImage>,
    ) -> Result<(), (u64, u64)> {
        let actual = image_bytes(&image);
        let prior = self.prior_bytes(&url);
        let per_available = self.remaining_for(&url);
        let previous = self.entries.get(&url);
        let svg = matches!(
            previous.map_or(&source, |entry| &entry.source),
            CachedBackgroundSource::Svg(_)
        );
        // A decoded raster image has one size; only SVG keeps several.
        let mut rasters: Vec<_> = match previous {
            Some(entry) if svg => entry.rasters.clone(),
            _ => Vec::new(),
        };
        rasters.retain(|(cached, _)| *cached != size);
        rasters.push((size, image));
        let excess = rasters.len().saturating_sub(MAX_RASTER_SIZES);
        rasters.drain(..excess);
        loop {
            let total = rasters.iter().fold(0, |sum: u64, (_, image)| {
                sum.saturating_add(image_bytes(image))
            });
            let fits = total <= per_available
                && (total <= prior || self.process_budget.try_reserve(total - prior));
            if fits {
                if prior > total {
                    self.process_budget.release(prior - total);
                }
                self.decoded_bytes = self
                    .decoded_bytes
                    .saturating_sub(prior)
                    .saturating_add(total);
                break;
            }
            if rasters.len() == 1 {
                let limit = if actual > per_available {
                    per_available
                } else {
                    self.process_budget.available_for(prior).min(per_available)
                };
                return Err((limit, actual));
            }
            rasters.remove(0);
        }
        let retained_source = self
            .entries
            .remove(&url)
            .map(|entry| entry.source)
            .unwrap_or(source);
        self.entries.insert(
            url,
            CachedBackgroundImage {
                source: retained_source,
                rasters,
            },
        );
        Ok(())
    }
}
