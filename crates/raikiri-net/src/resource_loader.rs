//! Resource loading primitives used by image consumers.
//!
//! URL-scheme handling and `data:` decoding belong in this resource layer,
//! before bytes reach a format decoder. This keeps `ImageResolver` as a thin
//! replaced-element adapter and leaves the decoder independent of URLs and
//! providers.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use raikiri_svg::{SvgDocument, SvgRootStyle, SvgViewport};
use raikiri_traits::vector::VectorImage;
use raikiri_traits::{
    DecodedImage, FetchedResource, ImageIntrinsicSize, ImageRasterSize, NetworkProvider, Request,
    ResolverError, policy::ResourceKind,
};
use url::{Position, Url};

use crate::image_decoder::ImageDecoder;

/// Maximum encoded image input and SVG raster output size accepted by this adapter.
pub(crate) const MAX_SVG_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CONCURRENT_LOADS: usize = 4;
/// SVG rasters kept per source, one per paint size, so that an image painted
/// at a few sizes in turn is not rasterized again at every switch. SVG scenes
/// are kept the same way.
const MAX_RASTER_SIZES: usize = 4;

struct CacheBudget {
    used: u64,
    limit: u64,
}

impl CacheBudget {
    fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }
}

fn cache_limit_error() -> ResolverError {
    ResolverError::Decode("image cache resource limit exceeded".to_owned())
}

/// A conservative source identity for HTTP raster images. Structured and
/// encoded fragments retain their display identity, including media fragments.
fn raster_source_key(url: &Url) -> Option<&str> {
    if !matches!(url.scheme(), "http" | "https")
        || !url
            .fragment()
            .unwrap_or_default()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'-'))
    {
        return None;
    }
    Some(&url[..Position::AfterQuery])
}

#[derive(Default)]
struct SourceCache {
    // Separate namespaces prevent a fragmentless SVG from being overwritten
    // when another response at a fragment alias turns out to be a raster.
    exact: HashMap<String, Arc<ImageSource>>,
    rasters: HashMap<String, Arc<ImageSource>>,
}

impl SourceCache {
    fn get(&self, url: &Url) -> Option<&Arc<ImageSource>> {
        self.exact
            .get(url.as_str())
            .or_else(|| self.rasters.get(raster_source_key(url)?))
    }

    fn len(&self) -> usize {
        self.exact.len() + self.rasters.len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn reserve(&mut self, additional: usize) {
        self.exact.reserve(additional);
        self.rasters.reserve(additional);
    }

    fn insert(&mut self, key: String, source: Arc<ImageSource>, shared: bool) {
        let cache = if shared {
            &mut self.rasters
        } else {
            &mut self.exact
        };
        cache.insert(key, source);
    }
}

#[derive(Default)]
struct LoadState {
    cache: SourceCache,
    loading: HashSet<String>,
}

enum Admission<'a> {
    Cached(Arc<ImageSource>),
    Pending(PendingSource<'a>),
}

struct PendingSource<'a> {
    state: &'a Mutex<LoadState>,
    budget: &'a Mutex<CacheBudget>,
    changed: &'a Condvar,
    url: &'a str,
    key_bytes: u64,
    active: bool,
}

impl Drop for PendingSource<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        // Only release this guard's reservation. Poison remains set, so normal
        // admission still fails closed even if an unrelated operation panicked.
        let mut budget = self
            .budget
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.loading.remove(self.url) {
            budget.used = budget.used.saturating_sub(self.key_bytes);
        }
        drop(state);
        drop(budget);
        self.changed.notify_all();
    }
}

/// Loads and caches decoded image resources.
///
/// `ResourceLoader` owns provider dispatch, `data:` URL decoding, request
/// construction, and the decoded-image cache. [`ImageDecoder`] remains a pure
/// bytes-to-pixels component.
pub(crate) struct ResourceLoader<N> {
    network: N,
    decoder: ImageDecoder,
    state: Mutex<LoadState>,
    changed: Condvar,
    budget: Arc<Mutex<CacheBudget>>,
    max_entries: usize,
    max_in_flight: usize,
}

pub(crate) struct ImageSource {
    kind: ImageSourceKind,
    pub(crate) intrinsic: ImageIntrinsicSize,
    /// SVG rasters by paint size, least recently used first.
    raster_cache: Mutex<Vec<(ImageRasterSize, Arc<DecodedImage>)>>,
    /// SVG scenes by box size, least recently used first.
    scene_cache: Mutex<Vec<(ImageRasterSize, Arc<VectorImage>)>>,
    /// Set once the scene builder has sent the document to the raster path,
    /// so no other box asks for a scene again.
    raster_only: AtomicBool,
    budget: Arc<Mutex<CacheBudget>>,
}

enum ImageSourceKind {
    Raster(Arc<DecodedImage>),
    Svg(Arc<[u8]>),
}

impl ImageSource {
    fn raster(image: Arc<DecodedImage>, budget: Arc<Mutex<CacheBudget>>) -> Self {
        let width = image.width as f32;
        let height = image.height as f32;
        Self {
            kind: ImageSourceKind::Raster(image),
            intrinsic: ImageIntrinsicSize::new(
                Some(width),
                Some(height),
                (height > 0.0).then_some(width / height),
            ),
            raster_cache: Mutex::new(Vec::new()),
            scene_cache: Mutex::new(Vec::new()),
            raster_only: AtomicBool::new(false),
            budget,
        }
    }

    fn svg(document: SvgDocument, bytes: Arc<[u8]>, budget: Arc<Mutex<CacheBudget>>) -> Self {
        let intrinsic = document.intrinsic_size();
        Self {
            // Retain only charged source bytes. SVG use references can expand a
            // small input into a much larger parsed tree.
            kind: ImageSourceKind::Svg(bytes),
            intrinsic: ImageIntrinsicSize::new(
                intrinsic.width,
                intrinsic.height,
                intrinsic.aspect_ratio,
            ),
            raster_cache: Mutex::new(Vec::new()),
            scene_cache: Mutex::new(Vec::new()),
            raster_only: AtomicBool::new(false),
            budget,
        }
    }

    /// The scene of an SVG source for a box of `size`, built once per size
    /// and kept under the byte budget of the pixels.
    ///
    /// `None` for a source that is not SVG, for a document the scene builder
    /// leaves to the raster path (remembered, so it is asked once), and for a
    /// document or size it cannot build a scene for. The caller asks for
    /// pixels next. The document is built with the default style and without
    /// fonts, as its raster is.
    pub(crate) fn scene(&self, size: ImageRasterSize) -> Option<Arc<VectorImage>> {
        let ImageSourceKind::Svg(bytes) = &self.kind else {
            return None;
        };
        if self.raster_only.load(Ordering::Relaxed) {
            return None;
        }
        {
            let mut cached = self.scene_cache.lock().ok()?;
            if let Some(index) = cached.iter().position(|(cached, _)| *cached == size) {
                let entry = cached.remove(index);
                let scene = entry.1.clone();
                cached.push(entry);
                return Some(scene);
            }
        }
        let document = SvgDocument::parse(bytes).ok()?;
        let scene = match document.scene(
            SvgViewport {
                width: size.width,
                height: size.height,
            },
            SvgRootStyle::default(),
        ) {
            Ok(scene) => Arc::new(scene),
            Err(error) => {
                if error.falls_back_to_raster() {
                    self.raster_only.store(true, Ordering::Relaxed);
                }
                return None;
            }
        };
        let new_bytes = u64::try_from(scene.estimated_heap_bytes()).unwrap_or(u64::MAX);
        let mut cached = self.scene_cache.lock().ok()?;
        let mut budget = self.budget.lock().ok()?;
        // Make room for the new scene by dropping the least recently used
        // sizes. A scene that cannot fit even then is returned without being
        // kept, and built again the next time it is asked for.
        let reclaimable: u64 = cached
            .iter()
            .map(|(_, scene)| u64::try_from(scene.estimated_heap_bytes()).unwrap_or(u64::MAX))
            .fold(0, u64::saturating_add);
        if new_bytes > budget.remaining().saturating_add(reclaimable) {
            return Some(scene);
        }
        let mut excess = new_bytes.saturating_sub(budget.remaining());
        while !cached.is_empty() && (cached.len() >= MAX_RASTER_SIZES || excess > 0) {
            let (_, old) = cached.remove(0);
            let old_bytes = u64::try_from(old.estimated_heap_bytes()).unwrap_or(u64::MAX);
            excess = excess.saturating_sub(old_bytes);
            budget.used = budget.used.saturating_sub(old_bytes);
        }
        budget.used = budget.used.saturating_add(new_bytes);
        cached.push((size, scene.clone()));
        Some(scene)
    }

    pub(crate) fn rasterize(
        &self,
        size: ImageRasterSize,
        max_output_bytes: Option<u64>,
    ) -> Option<Arc<DecodedImage>> {
        match &self.kind {
            ImageSourceKind::Raster(image) => {
                if max_output_bytes.is_some_and(|limit| image.rgba.len() as u64 > limit) {
                    None
                } else {
                    Some(image.clone())
                }
            }
            ImageSourceKind::Svg(bytes) => {
                let mut cached = self.raster_cache.lock().ok()?;
                if let Some(index) = cached.iter().position(|(cached, _)| *cached == size) {
                    let entry = cached.remove(index);
                    let image = entry.1.clone();
                    cached.push(entry);
                    return max_output_bytes
                        .is_none_or(|limit| image.rgba.len() as u64 <= limit)
                        .then_some(image);
                }
                let cached_bytes: u64 = cached
                    .iter()
                    .map(|(_, image)| image.rgba.len() as u64)
                    .sum();
                // Serialize allocations across all sources sharing this cache.
                // Keep the previous rasters on failure. The new raster may
                // use the bytes of every cached size, which are then evicted,
                // so the extra buffer is bounded by the SVG per-image output
                // limit.
                let mut budget = self.budget.lock().ok()?;
                let available = budget.remaining().checked_add(cached_bytes)?;
                let limit = max_output_bytes.map_or(available, |limit| limit.min(available));
                let document = SvgDocument::parse(bytes).ok()?;
                let image = document
                    .rasterize(
                        SvgViewport {
                            width: size.width,
                            height: size.height,
                        },
                        SvgRootStyle::default(),
                        Some(limit),
                    )
                    .ok()?;
                let image = Arc::new(image);
                let new_bytes = image.rgba.len() as u64;
                // Evict the least recently used sizes until the new raster
                // fits the budget and the size count.
                let mut excess = new_bytes.saturating_sub(budget.remaining());
                while !cached.is_empty() && (cached.len() >= MAX_RASTER_SIZES || excess > 0) {
                    let (_, old) = cached.remove(0);
                    let old_bytes = old.rgba.len() as u64;
                    excess = excess.saturating_sub(old_bytes);
                    budget.used = budget.used.checked_sub(old_bytes)?;
                }
                budget.used += new_bytes;
                cached.push((size, image.clone()));
                Some(image)
            }
        }
    }

    pub(crate) fn decoded_byte_len(&self) -> Option<u64> {
        match &self.kind {
            ImageSourceKind::Raster(image) => Some(image.rgba.len() as u64),
            ImageSourceKind::Svg(_) => None,
        }
    }

    #[cfg(test)]
    fn cached_raster(&self) -> Option<Arc<DecodedImage>> {
        match &self.kind {
            ImageSourceKind::Raster(image) => Some(image.clone()),
            ImageSourceKind::Svg(_) => self
                .raster_cache
                .lock()
                .ok()?
                .last()
                .map(|(_, image)| image.clone()),
        }
    }
}

impl<N: NetworkProvider> ResourceLoader<N> {
    pub(crate) fn new(network: N) -> Self {
        Self::with_limits(network, MAX_CACHE_BYTES, MAX_CACHE_ENTRIES)
    }

    fn with_limits(network: N, max_bytes: u64, max_entries: usize) -> Self {
        Self::with_limits_and_concurrency(network, max_bytes, max_entries, MAX_CONCURRENT_LOADS)
    }

    fn with_limits_and_concurrency(
        network: N,
        max_bytes: u64,
        max_entries: usize,
        max_in_flight: usize,
    ) -> Self {
        Self {
            network,
            decoder: ImageDecoder,
            state: Mutex::new(LoadState::default()),
            changed: Condvar::new(),
            budget: Arc::new(Mutex::new(CacheBudget {
                used: 0,
                limit: max_bytes,
            })),
            max_entries,
            max_in_flight: max_in_flight.max(1),
        }
    }

    /// Fetches, decodes/parses, and caches one image resource.
    #[cfg(test)]
    pub(crate) fn load(&self, url: &Url) -> Result<Arc<DecodedImage>, ResolverError> {
        let source = self.load_source(url)?;
        if let Some(image) = source.cached_raster() {
            return Ok(image);
        }
        let size = default_object_size(source.intrinsic);
        source
            .rasterize(size, Some(MAX_SVG_BYTES as u64))
            .ok_or_else(|| ResolverError::Decode("SVG rasterization failed".to_owned()))
    }

    #[cfg(test)]
    pub(crate) fn cached(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        self.cached_source(url)?.cached_raster()
    }

    pub(crate) fn cached_source(&self, url: &Url) -> Option<Arc<ImageSource>> {
        self.state.lock().ok()?.cache.get(url).cloned()
    }

    fn wait_for_change(&self, state: MutexGuard<'_, LoadState>) -> Result<(), ResolverError> {
        drop(self.changed.wait(state).map_err(|_| cache_limit_error())?);
        Ok(())
    }

    fn admit<'a>(&'a self, url: &'a Url) -> Result<Admission<'a>, ResolverError> {
        if let Some(cached) = self.cached_source(url) {
            return Ok(Admission::Cached(cached));
        }
        // The eventual format is unknown: reserve enough for an exact SVG
        // key, even when the pending key is a borrowed fragmentless prefix.
        let pending_key = raster_source_key(url).unwrap_or(url.as_str());
        let key_bytes = url.as_str().len() as u64;
        loop {
            // Budget precedes State everywhere they are held together. Cache
            // hits take State alone; provider calls hold neither mutex.
            let mut budget = self.budget.lock().map_err(|_| cache_limit_error())?;
            let mut state = self.state.lock().map_err(|_| cache_limit_error())?;
            if let Some(cached) = state.cache.get(url) {
                return Ok(Admission::Cached(cached.clone()));
            }
            // A duplicate waits for its owner even when that reservation fills
            // the entry or byte budget. It consumes no additional permit/key.
            if state.loading.contains(pending_key) {
                drop(budget);
                self.wait_for_change(state)?;
                continue;
            }
            if state.cache.len() >= self.max_entries {
                return Err(cache_limit_error());
            }
            if state.loading.len() >= self.max_in_flight
                || state.cache.len() + state.loading.len() >= self.max_entries
            {
                drop(budget);
                self.wait_for_change(state)?;
                continue;
            }
            if key_bytes > budget.remaining() {
                return Err(cache_limit_error());
            }
            // Prepare allocations before charging the key. Reserve room for
            // every pending source's eventual cache insertion, too.
            let key = pending_key.to_owned();
            let pending = state.loading.len();
            state.cache.reserve(pending + 1);
            state.loading.reserve(1);
            budget.used += key_bytes;
            state.loading.insert(key);
            return Ok(Admission::Pending(PendingSource {
                state: &self.state,
                budget: &self.budget,
                changed: &self.changed,
                url: pending_key,
                key_bytes,
                active: true,
            }));
        }
    }

    pub(crate) fn load_source(&self, url: &Url) -> Result<Arc<ImageSource>, ResolverError> {
        self.load_source_with(url, |bytes, is_data_url, is_svg, available| {
            self.build_source(bytes, is_data_url, is_svg, available)
        })
    }

    fn load_source_with(
        &self,
        url: &Url,
        build: impl FnOnce(Vec<u8>, bool, bool, u64) -> Result<Arc<ImageSource>, ResolverError>,
    ) -> Result<Arc<ImageSource>, ResolverError> {
        let mut admission = match self.admit(url)? {
            Admission::Cached(source) => return Ok(source),
            Admission::Pending(admission) => admission,
        };
        // Keep the permit until decode and publication finish, bounding the
        // number of fetched buffers waiting for the serialized decoder.
        let (bytes, is_data_url, is_svg, final_url_shareable) = self.fetch_bytes(url)?;
        let source_bytes = if is_svg { bytes.len() as u64 } else { 0 };
        let mut budget = self.budget.lock().map_err(|_| cache_limit_error())?;
        let available = budget
            .remaining()
            .checked_sub(source_bytes)
            .ok_or_else(cache_limit_error)?;
        // The builder only constructs an uncommitted source. Catch its unwind
        // inside this guard's lifetime so the shared budget is not poisoned.
        let built = catch_unwind(AssertUnwindSafe(|| {
            build(bytes, is_data_url, is_svg, available)
        }));
        let source = match built {
            Ok(source) => source?,
            Err(panic) => {
                drop(budget);
                resume_unwind(panic);
            }
        };
        let charge = source_bytes
            .checked_add(source.decoded_byte_len().unwrap_or(0))
            .ok_or_else(cache_limit_error)?;
        let shared = final_url_shareable
            && raster_source_key(url).is_some()
            && matches!(source.kind, ImageSourceKind::Raster(_));
        let retained_key_bytes = if shared {
            admission.url.len()
        } else {
            url.as_str().len()
        } as u64;
        let refund = admission.key_bytes - retained_key_bytes;
        let used = budget
            .used
            .checked_sub(refund)
            .and_then(|used| used.checked_add(charge))
            .filter(|&used| used <= budget.limit)
            .ok_or_else(cache_limit_error)?;
        let mut state = self.state.lock().map_err(|_| cache_limit_error())?;
        // Move the one owned pending key into its final namespace. An exact
        // source needs the original suffix, whose bytes were reserved up front.
        let mut key = state
            .loading
            .take(admission.url)
            .expect("pending source owns its key");
        if !shared {
            key.reserve_exact(url.as_str().len() - key.len());
            key.push_str(&url.as_str()[key.len()..]);
        }
        state.cache.insert(key, source.clone(), shared);
        budget.used = used;
        admission.active = false;
        drop(state);
        drop(budget);
        self.changed.notify_all();
        Ok(source)
    }

    fn fetch_bytes(&self, url: &Url) -> Result<(Vec<u8>, bool, bool, bool), ResolverError> {
        let result = if url.scheme() == "data" {
            let is_svg = data_url_media_type(url.as_str()).is_some_and(is_svg_media_type);
            (decode_data_url(url.as_str())?, true, is_svg, false)
        } else {
            let fetched = self.fetch(url)?;
            if fetched.bytes.len() > MAX_SVG_BYTES {
                return Err(ResolverError::Decode(
                    "image input exceeds byte limit".to_owned(),
                ));
            }
            let is_svg = fetched
                .content_type
                .as_deref()
                .is_some_and(is_svg_media_type)
                || url.path().to_ascii_lowercase().ends_with(".svg");
            (
                fetched.bytes.to_vec(),
                false,
                is_svg,
                raster_source_key(&fetched.final_url).is_some(),
            )
        };
        if result.0.len() > MAX_SVG_BYTES {
            return Err(ResolverError::Decode(
                "image input exceeds byte limit".to_owned(),
            ));
        }
        Ok(result)
    }

    fn build_source(
        &self,
        bytes: Vec<u8>,
        is_data_url: bool,
        is_svg: bool,
        available: u64,
    ) -> Result<Arc<ImageSource>, ResolverError> {
        let source = if is_svg {
            let document = SvgDocument::parse(&bytes)
                .map_err(|error| ResolverError::Decode(format!("SVG parse failed: {error}")))?;
            ImageSource::svg(document, bytes.into(), self.budget.clone())
        } else {
            let decoded = self.decoder.decode(&bytes, available).map_err(|error| {
                if is_data_url {
                    ResolverError::Decode(format!("data URL image decode failed: {error}"))
                } else {
                    ResolverError::Decode(error)
                }
            })?;
            ImageSource::raster(Arc::new(decoded), self.budget.clone())
        };
        Ok(Arc::new(source))
    }

    fn fetch(&self, url: &Url) -> Result<FetchedResource, ResolverError> {
        self.network
            .fetch(Request::new(url.clone(), ResourceKind::Image))
            .map_err(ResolverError::Network)
    }
}

pub(crate) fn default_object_size(natural: ImageIntrinsicSize) -> ImageRasterSize {
    const DEFAULT_WIDTH: f32 = 300.0;
    const DEFAULT_HEIGHT: f32 = 150.0;

    match (natural.width, natural.height, natural.aspect_ratio) {
        (Some(width), Some(height), _) => ImageRasterSize { width, height },
        (Some(width), None, Some(ratio)) if ratio > 0.0 => ImageRasterSize {
            width,
            height: width / ratio,
        },
        (None, Some(height), Some(ratio)) if ratio > 0.0 => ImageRasterSize {
            width: height * ratio,
            height,
        },
        (None, None, Some(ratio)) if ratio > 0.0 => {
            let width = DEFAULT_WIDTH.min(DEFAULT_HEIGHT * ratio);
            ImageRasterSize {
                width,
                height: width / ratio,
            }
        }
        (Some(width), None, _) => ImageRasterSize {
            width,
            height: DEFAULT_HEIGHT,
        },
        (None, Some(height), _) => ImageRasterSize {
            width: DEFAULT_WIDTH,
            height,
        },
        _ => ImageRasterSize {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        },
    }
}

pub(crate) fn is_svg_media_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("image/svg+xml"))
}

fn data_url_media_type(value: &str) -> Option<&str> {
    let header = value.strip_prefix("data:")?.split_once(',')?.0;
    Some(header.split(';').next().unwrap_or(header))
}

/// Decodes the body of a `data:` URL in the resource layer.
fn decode_data_url(url: &str) -> Result<Vec<u8>, ResolverError> {
    let data_url = data_url::DataUrl::process(url)
        .map_err(|error| ResolverError::Decode(format!("invalid data URL: {error}")))?;
    let (bytes, _fragment) = data_url
        .decode_to_vec()
        .map_err(|error| ResolverError::Decode(format!("invalid data URL payload: {error}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
