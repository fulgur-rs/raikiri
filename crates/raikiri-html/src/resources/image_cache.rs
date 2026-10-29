//! Decoded background image caches and the process-wide decode budget
//! shared by every resource set.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use raikiri_svg::SvgDocument;
use raikiri_traits::vector::VectorImage;
use raikiri_traits::{DecodedImage, ImageRasterSize, RenderWarning};
use url::Url;

use super::DEFAULT_MAX_DECODED_IMAGE_CACHE_BYTES;

#[cfg(feature = "cache-accounting-diagnostics")]
pub(super) mod probe;

/// Maximum CSS background image decodes that may run at once, process-wide.
///
/// Independent [`RenderResources`](super::RenderResources) instances share this semaphore, so decoder
/// concurrency stays bounded no matter how many resource contexts exist.
/// Unrelated images proceed concurrently up to this bound instead of sharing
/// one exclusive mutex; only work beyond the bound waits.
pub(super) const PROCESS_MAX_CONCURRENT_IMAGE_DECODES: usize = 4;
/// Maximum accounted CSS background cache bytes retained, process-wide.
///
/// Each resource set keeps its own 128 MiB cache, but the sum across all live
/// sets is additionally capped here. This bounds cache ownership, not
/// allocations still held by callers after cache eviction or drop. Inserts that would exceed the process cap fail
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

/// Most scenes of SVG images that a resource set keeps at once.
pub(super) const MAX_CACHED_SCENES: usize = 16;

/// A scene of an SVG image, built for one box.
struct CachedScene {
    url: Url,
    size: ImageRasterSize,
    scene: Arc<VectorImage>,
    bytes: u64,
}

fn image_bytes(image: &DecodedImage) -> u64 {
    u64::try_from(image.rgba.len()).unwrap_or(u64::MAX)
}

/// Process-wide budget shared by every [`RenderResources`](super::RenderResources) for CSS background decoding.
///
/// The per-set 128 MiB cache bounds one security context; this object bounds
/// their sum plus concurrent decoder slots process-wide. Reconfigured
/// [`RenderResources`](super::RenderResources) clones keep sharing the same budget object while their
/// caches, provider budgets, and in-flight maps are replaced, so cache and
/// policy isolation holds while decoder concurrency and cache reservations stay
/// bounded across independent contexts. Caller-held allocations after eviction
/// or cache drop are outside this accounting.
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

    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
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

    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
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
    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
    count_accounting_work: std::cell::Cell<bool>,
    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
    accounting_work: std::cell::Cell<u64>,
    pub(super) entries: HashMap<Url, CachedBackgroundImage>,
    pub(super) decoded_bytes: u64,
    /// The scenes of SVG images, oldest first. They share the byte budgets of
    /// the rasters, so an image kept as both pays for both.
    scenes: Vec<CachedScene>,
    scene_bytes: u64,
    /// The SVG images whose scene builder asked for the raster path, which no
    /// box changes: asking again would only repeat the work.
    raster_only: HashSet<Url>,
    pub(super) max_bytes: u64,
    pub(super) process_budget: Arc<ProcessImageBudget>,
}

impl DecodedImageCache {
    pub(super) fn new(process_budget: Arc<ProcessImageBudget>) -> Self {
        Self {
            #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
            count_accounting_work: std::cell::Cell::new(false),
            #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
            accounting_work: std::cell::Cell::new(0),
            entries: HashMap::new(),
            decoded_bytes: 0,
            scenes: Vec::new(),
            scene_bytes: 0,
            raster_only: HashSet::new(),
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
        self.process_budget
            .release(self.decoded_bytes.saturating_add(self.scene_bytes));
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

    /// Source allocations are counted once per Arc identity; each SVG URL
    /// also pays for its entry and key storage. Backend trees are never kept.
    fn source_bytes<'a>(
        &self,
        entries: impl Iterator<Item = (&'a Url, &'a CachedBackgroundImage)>,
    ) -> u64 {
        let mut seen = HashSet::new();
        entries.fold(0_u64, |sum, (url, entry)| {
            #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
            if self.count_accounting_work.get() {
                self.accounting_work
                    .set(self.accounting_work.get().saturating_add(1));
            }
            let CachedBackgroundSource::Svg(document) = &entry.source else {
                return sum;
            };
            let metadata = (std::mem::size_of::<CachedBackgroundImage>()
                + std::mem::size_of::<Url>()
                + 2 * std::mem::size_of::<usize>()) as u64
                + url.as_str().len() as u64;
            let allocation = if seen.insert(Arc::as_ptr(document)) {
                document
                    .source_only_retained_bytes()
                    .expect("SVG cache sources never retain a backend tree")
            } else {
                0
            };
            sum.saturating_add(metadata).saturating_add(allocation)
        })
    }

    pub(super) fn remaining_for(&self, url: &Url) -> u64 {
        let existing = self.prior_bytes(url);
        self.max_bytes.saturating_sub(
            self.decoded_bytes
                .saturating_sub(existing)
                .saturating_add(self.scene_bytes),
        )
    }

    /// The scene of the SVG image at `url` that was built for `size`.
    pub(super) fn scene(&self, url: &Url, size: ImageRasterSize) -> Option<Arc<VectorImage>> {
        self.scenes
            .iter()
            .find(|cached| cached.size == size && &cached.url == url)
            .map(|cached| Arc::clone(&cached.scene))
    }

    /// Whether the scene builder asked for the raster path for `url`.
    pub(super) fn is_raster_only(&self, url: &Url) -> bool {
        self.raster_only.contains(url)
    }

    /// Notes that `url` can only be drawn as pixels.
    pub(super) fn mark_raster_only(&mut self, url: Url) {
        self.raster_only.insert(url);
    }

    /// Keeps `scene` as the scene of `url` for `size`, when its heap bytes fit
    /// the budgets once the oldest scenes make room. A scene that cannot be
    /// kept is still good for the caller that built it; it is built again
    /// when it is asked for again.
    pub(super) fn insert_scene(
        &mut self,
        url: Url,
        size: ImageRasterSize,
        scene: Arc<VectorImage>,
    ) {
        let bytes = u64::try_from(scene.estimated_heap_bytes()).unwrap_or(u64::MAX);
        self.remove_scene(&url, size);
        // A scene the rasters leave no room for would only push the others out.
        if self.decoded_bytes.saturating_add(bytes) > self.max_bytes {
            return;
        }
        while self.scenes.len() >= MAX_CACHED_SCENES
            || (!self.scene_fits(bytes) && !self.scenes.is_empty())
        {
            self.release_scene(0);
        }
        if !self.scene_fits(bytes) || !self.process_budget.try_reserve(bytes) {
            return;
        }
        self.scene_bytes = self.scene_bytes.saturating_add(bytes);
        self.scenes.push(CachedScene {
            url,
            size,
            scene,
            bytes,
        });
    }

    /// Whether `bytes` more of scene fit this set's byte budget.
    fn scene_fits(&self, bytes: u64) -> bool {
        self.decoded_bytes
            .saturating_add(self.scene_bytes)
            .saturating_add(bytes)
            <= self.max_bytes
    }

    fn remove_scene(&mut self, url: &Url, size: ImageRasterSize) {
        if let Some(at) = self
            .scenes
            .iter()
            .position(|cached| cached.size == size && &cached.url == url)
        {
            self.release_scene(at);
        }
    }

    fn release_scene(&mut self, at: usize) {
        let removed = self.scenes.remove(at);
        self.scene_bytes = self.scene_bytes.saturating_sub(removed.bytes);
        self.process_budget.release(removed.bytes);
    }

    pub(super) fn insert_source(
        &mut self,
        url: Url,
        source: CachedBackgroundSource,
    ) -> Result<(), (u64, u64)> {
        let source = match source {
            CachedBackgroundSource::Svg(document) => {
                CachedBackgroundSource::Svg(document.into_source_only())
            }
            raster => raster,
        };
        let replacement = CachedBackgroundImage {
            source,
            rasters: Vec::new(),
        };
        let old_sources = self.source_bytes(self.entries.iter());
        let new_sources = self.source_bytes(
            self.entries
                .iter()
                .filter(|(key, _)| *key != &url)
                .chain(std::iter::once((&url, &replacement))),
        );
        let prior = self.prior_bytes(&url).saturating_add(old_sources);
        let next = self
            .decoded_bytes
            .saturating_sub(prior)
            .saturating_add(new_sources)
            .saturating_add(Self::raster_bytes(&replacement));
        let limit = self.max_bytes.saturating_sub(self.scene_bytes);
        if next > limit {
            return Err((limit, next));
        }
        if next > self.decoded_bytes && !self.process_budget.try_reserve(next - self.decoded_bytes)
        {
            return Err((self.process_budget.available_for(self.decoded_bytes), next));
        }
        if next < self.decoded_bytes {
            self.process_budget.release(self.decoded_bytes - next);
        }
        self.decoded_bytes = next;
        self.raster_only.remove(&url);
        while let Some(at) = self.scenes.iter().position(|cached| cached.url == url) {
            self.release_scene(at);
        }
        self.entries.insert(url, replacement);
        Ok(())
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
        if !self.entries.contains_key(&url) && matches!(source, CachedBackgroundSource::Svg(_)) {
            self.insert_source(url.clone(), source.clone())?;
        }
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
