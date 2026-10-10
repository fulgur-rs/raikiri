//! Renderer-neutral resource configuration shared by parse and render entry points.

use std::collections::HashMap;
use std::fmt;
use std::io::{Cursor, Read};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Instant;

use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use raikiri_dom::FontFaceLoader;
use raikiri_style::{
    ComputedValues, PageCascadeResult,
    property::{BackgroundImage, DisplayValue, PropertyKey, PropertyValue, Visibility},
};
use raikiri_svg::{SvgDocument, SvgRootStyle, SvgViewport};
use raikiri_traits::{
    AbortSignal, Body, DecodedImage, FetchOutcome, ImageIntrinsicSize, ImageRasterSize, Method,
    NetworkError, NetworkProvider, PolicyViolation, RenderLimits, RenderWarning, Request,
    ResourceKind, ResourcePolicy, ViolationType, WarningKind,
};
use url::Url;

use raikiri_traits::{ImagePixelSource, ReplacedResolver};

use crate::{HtmlDocument, ParseOptions};

/// Default maximum response size accepted through [`RenderResources`].
pub const DEFAULT_MAX_RESOURCE_BYTES: u64 = 32 * 1024 * 1024;
/// Default maximum combined response bytes accepted through [`RenderResources`].
pub const DEFAULT_MAX_AGGREGATE_RESOURCE_BYTES: u64 = 128 * 1024 * 1024;
/// Maximum decoded CSS background image bytes retained by one resource set.
const DEFAULT_MAX_DECODED_IMAGE_CACHE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_BACKGROUND_IMAGE_ATTEMPTS: usize = 256;
const MAX_SVG_IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// Maximum CSS background image decodes that may run at once, process-wide.
///
/// Independent [`RenderResources`] instances share this semaphore, so decoder
/// concurrency stays bounded no matter how many resource contexts exist.
/// Unrelated images proceed concurrently up to this bound instead of sharing
/// one exclusive mutex; only work beyond the bound waits.
const PROCESS_MAX_CONCURRENT_IMAGE_DECODES: usize = 4;
/// Maximum decoded CSS background raster bytes retained, process-wide.
///
/// Each resource set keeps its own 128 MiB cache, but the sum across all live
/// sets is additionally capped here so independent contexts cannot grow the
/// process peak without bound. Inserts that would exceed the process cap fail
/// with the same per-image limit warning the per-set cap produces.
const PROCESS_MAX_RETAINED_DECODED_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
enum CachedBackgroundSource {
    Raster(Arc<DecodedImage>),
    Svg(Arc<SvgDocument>),
}

#[derive(Clone)]
struct CachedBackgroundImage {
    source: CachedBackgroundSource,
    raster: Option<(ImageRasterSize, Arc<DecodedImage>)>,
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
    fn global() -> Arc<Self> {
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

    fn available_for(&self, prior: u64) -> u64 {
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

    fn acquire(self: &Arc<Self>) -> ProcessDecodeGuard {
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
    fn try_acquire(self: &Arc<Self>) -> Option<ProcessDecodeGuard> {
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

struct BackgroundInflight {
    state: Mutex<InflightState>,
    cvar: Condvar,
}

impl BackgroundInflight {
    fn new() -> Self {
        Self {
            state: Mutex::new(InflightState {
                done: false,
                warning: None,
            }),
            cvar: Condvar::new(),
        }
    }

    fn wait_for_done(&self) -> Option<RenderWarning> {
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

    fn finish(&self, warning: Option<RenderWarning>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.warning = warning;
        state.done = true;
        self.cvar.notify_all();
    }
}

struct DecodedImageCache {
    entries: HashMap<Url, CachedBackgroundImage>,
    decoded_bytes: u64,
    max_bytes: u64,
    process_budget: Arc<ProcessImageBudget>,
}

impl DecodedImageCache {
    fn new(process_budget: Arc<ProcessImageBudget>) -> Self {
        Self {
            entries: HashMap::new(),
            decoded_bytes: 0,
            max_bytes: DEFAULT_MAX_DECODED_IMAGE_CACHE_BYTES,
            process_budget,
        }
    }

    fn prior_bytes(&self, url: &Url) -> u64 {
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
        entry
            .raster
            .as_ref()
            .map(|(_, image)| u64::try_from(image.rgba.len()).unwrap_or(u64::MAX))
            .or_else(|| match &entry.source {
                CachedBackgroundSource::Raster(image) => {
                    Some(u64::try_from(image.rgba.len()).unwrap_or(u64::MAX))
                }
                CachedBackgroundSource::Svg(_) => None,
            })
            .unwrap_or(0)
    }

    fn remaining_for(&self, url: &Url) -> u64 {
        let existing = self.prior_bytes(url);
        self.max_bytes
            .saturating_sub(self.decoded_bytes.saturating_sub(existing))
    }

    fn insert_source(&mut self, url: Url, source: CachedBackgroundSource) {
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
                raster: None,
            },
        );
    }

    fn insert_raster(
        &mut self,
        url: Url,
        source: CachedBackgroundSource,
        size: ImageRasterSize,
        image: Arc<DecodedImage>,
    ) -> Result<(), (u64, u64)> {
        let actual = u64::try_from(image.rgba.len()).unwrap_or(u64::MAX);
        let prior = self.prior_bytes(&url);
        let per_available = self.remaining_for(&url);
        if actual > per_available {
            return Err((per_available, actual));
        }
        if actual > prior {
            let delta = actual.saturating_sub(prior);
            if !self.process_budget.try_reserve(delta) {
                let global_available = self.process_budget.available_for(prior);
                return Err((global_available.min(per_available), actual));
            }
        } else if prior > actual {
            self.process_budget.release(prior.saturating_sub(actual));
        }
        let previous = self.entries.remove(&url);
        let retained_source = previous.map(|entry| entry.source).unwrap_or(source);
        self.decoded_bytes = self
            .decoded_bytes
            .saturating_sub(prior)
            .saturating_add(actual);
        self.entries.insert(
            url,
            CachedBackgroundImage {
                source: retained_source,
                raster: Some((size, image)),
            },
        );
        Ok(())
    }
}

/// Network response byte limits for one render operation.
///
/// These limits apply to responses after the consumer's [`NetworkProvider`]
/// returns. The provider must enforce transfer-time limits itself if it needs
/// to prevent buffering an oversized response before it reaches Raikiri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceLimits {
    /// Maximum response bytes for one request. Defaults to 32 MiB.
    pub max_resource_bytes: Option<u64>,
    /// Maximum response bytes across stylesheet, font, and image requests. Defaults to 128 MiB.
    pub max_aggregate_resource_bytes: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_resource_bytes: Some(DEFAULT_MAX_RESOURCE_BYTES),
            max_aggregate_resource_bytes: Some(DEFAULT_MAX_AGGREGATE_RESOURCE_BYTES),
        }
    }
}

impl ResourceLimits {
    /// Create limits with the fail-closed defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum response size. `None` disables this cap.
    pub fn max_resource_bytes(mut self, limit: Option<u64>) -> Self {
        self.max_resource_bytes = limit;
        self
    }

    /// Set the total response-byte budget. `None` disables this cap.
    pub fn max_aggregate_resource_bytes(mut self, limit: Option<u64>) -> Self {
        self.max_aggregate_resource_bytes = limit;
        self
    }
}

/// Consumer-supplied inputs and resource policy shared by parse and render.
///
/// Use the same value with [`crate::parse_html_with_resources`] and
/// [`crate::LayoutOptions::resources`] so both phases share stylesheet
/// sources, base URL, network provider, policy, fonts, resolver, and limits.
/// CSS background sources fetched during rendering are kept in an image cache
/// shared by clones, bounded to 128 MiB of decoded raster data per resource
/// set and 256 MiB retained process-wide across all live sets.
///
/// Concurrent renders sharing one security context (clones with the same
/// provider, policy, and limits) coordinate same-URL background fetches through
/// a shared in-flight map keyed by the fragment-stripped URL, so only one
/// provider request runs per URL. Unrelated URLs decode concurrently through a
/// process-wide semaphore with four slots instead of one exclusive mutex.
/// A synchronous decode cannot be interrupted mid-flight: an abort checked
/// before decoding skips the work, while an abort that fires mid-decode runs
/// to completion and may still cache (harmless content-addressed bytes; the
/// overall render still reports aborted). [`ResourcePolicy::decode_timeout`]
/// is enforced by discarding the result after the decode returns. In both
/// timed-out and aborted-during-decode cases the decoder slot is held until
/// that return, so work retains its budget until it stops. Fetch timeouts stay
/// with the consumer provider, which receives the same abort signal on every
/// background request.
///
/// Reconfiguring the provider, policy, replaced-resource source, or resource
/// limits replaces the cache, aggregate budget, and in-flight map, preserving
/// cache and policy isolation; the process-wide decode and retained budgets
/// stay shared because they bound the process, not one security context.
///
/// # Fonts
///
/// Text is laid out with the installed fonts
/// ([`raikiri_dom::system_font_collection`]) unless a font set is given with
/// [`fonts`](Self::fonts). Paragraphs are only built on several threads with
/// a font set built from bundled fonts only. For deterministic rendering,
/// build the fonts with [`crate::FontCollectionBuilder`], which disables
/// system font discovery by default and applies bundled fonts in a stable
/// fallback order.
#[derive(Clone)]
pub struct RenderResources<'a> {
    extra_stylesheets: Vec<String>,
    network: Option<&'a dyn NetworkProvider>,
    policy: Option<&'a dyn ResourcePolicy>,
    base_url: Option<Url>,
    font_collection: Option<shodo::font::FontCollection>,
    resolver: Option<&'a (dyn ReplacedResolver + Send + Sync)>,
    image_pixel_source: Option<&'a (dyn ImagePixelSource + Send + Sync)>,
    render_limits: RenderLimits,
    resource_limits: ResourceLimits,
    budget: Arc<Mutex<u64>>,
    image_cache: Arc<Mutex<DecodedImageCache>>,
    background_inflight: Arc<Mutex<HashMap<Url, Arc<BackgroundInflight>>>>,
    process_budget: Arc<ProcessImageBudget>,
}

impl fmt::Debug for RenderResources<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderResources")
            .field("extra_stylesheet_count", &self.extra_stylesheets.len())
            .field("has_network_provider", &self.network.is_some())
            .field("has_network_policy", &self.policy.is_some())
            .field("base_url", &self.base_url)
            .field("has_font_collection", &self.font_collection.is_some())
            .field("has_replaced_resolver", &self.resolver.is_some())
            .field("has_image_pixel_source", &self.image_pixel_source.is_some())
            .field("render_limits", &self.render_limits)
            .field("resource_limits", &self.resource_limits)
            .finish()
    }
}

impl<'a> Default for RenderResources<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> RenderResources<'a> {
    /// Create a resource configuration with no extra stylesheets or network
    /// provider, default render/resource limits, and the platform font context.
    pub fn new() -> Self {
        let process_budget = ProcessImageBudget::global();
        Self {
            extra_stylesheets: Vec::new(),
            network: None,
            policy: None,
            base_url: None,
            font_collection: None,
            resolver: None,
            image_pixel_source: None,
            render_limits: RenderLimits::default(),
            resource_limits: ResourceLimits::default(),
            budget: Arc::new(Mutex::new(0)),
            image_cache: Arc::new(Mutex::new(DecodedImageCache::new(Arc::clone(
                &process_budget,
            )))),
            background_inflight: Arc::new(Mutex::new(HashMap::new())),
            process_budget,
        }
    }

    fn reset_resource_state(&mut self) {
        self.budget = Arc::new(Mutex::new(0));
        self.image_cache = Arc::new(Mutex::new(DecodedImageCache::new(Arc::clone(
            &self.process_budget,
        ))));
        self.background_inflight = Arc::new(Mutex::new(HashMap::new()));
    }

    #[cfg(test)]
    pub(crate) fn with_test_process_budget(mut self, budget: Arc<ProcessImageBudget>) -> Self {
        self.process_budget = Arc::clone(&budget);
        self.image_cache = Arc::new(Mutex::new(DecodedImageCache::new(budget)));
        self.background_inflight = Arc::new(Mutex::new(HashMap::new()));
        self.budget = Arc::new(Mutex::new(0));
        self
    }

    /// Append a stylesheet source as a user stylesheet.
    pub fn stylesheet(mut self, source: impl Into<String>) -> Self {
        self.extra_stylesheets.push(source.into());
        self
    }

    /// Provide the consumer's synchronous network provider.
    pub fn network_provider(mut self, provider: &'a dyn NetworkProvider) -> Self {
        self.reset_resource_state();
        self.network = Some(provider);
        self
    }

    /// Apply this consumer policy to requests made through this configuration.
    ///
    /// Scheme, host, per-hop redirect, MIME, per-resource-size, and
    /// aggregate-size checks are applied around stylesheet, font, and CSS
    /// background image provider calls. Redirects are driven hop by hop via
    /// [`NetworkProvider::fetch_one_hop`] (not [`NetworkProvider::fetch`],
    /// which would already have followed every hop by the time it returns):
    /// each hop's target is checked against scheme/host policy and
    /// `allow_redirect`/`max_redirect_hops` *before* a request is made to
    /// it. The same scheme/host policy is checked before replaced-resource
    /// resolution, and `max_decoded_bytes(Image)` is checked before paint
    /// pixels are returned. The provider and any custom replaced-resource
    /// resolver remain responsible for transfer-time byte limits and
    /// fetch/decode timeouts because they materialize data before returning.
    pub fn network_policy(mut self, policy: &'a dyn ResourcePolicy) -> Self {
        self.reset_resource_state();
        self.policy = Some(policy);
        self
    }

    /// Set the fallback base URL for document-relative resources.
    ///
    /// A valid HTML `<base href>` takes precedence, consistently for stylesheets,
    /// `@font-face`, and replaced elements.
    pub fn base_url(mut self, base_url: Url) -> Self {
        self.base_url = Some(base_url);
        self
    }

    /// Use a font set built from bundled fonts for layout and paint.
    pub fn fonts(mut self, fonts: crate::RenderFonts) -> Self {
        self.font_collection = Some(fonts.into_collection());
        self
    }

    /// The font layer text is laid out with: the layer of the font set given
    /// to [`fonts`](Self::fonts), or the process-wide layer of the installed
    /// fonts ([`raikiri_dom::system_font_collection`]).
    pub fn inline_engine_fonts(&self) -> shodo::font::FontCollection {
        self.font_collection
            .clone()
            .unwrap_or_else(raikiri_dom::system_font_collection)
    }

    /// Set the limits used when parsing HTML.
    pub fn render_limits(mut self, limits: RenderLimits) -> Self {
        self.render_limits = limits;
        self
    }

    /// Set response-size limits for stylesheet, font, and image requests.
    pub fn resource_limits(mut self, limits: ResourceLimits) -> Self {
        self.reset_resource_state();
        self.resource_limits = limits;
        self
    }

    /// Use one cache/provider object for replaced-element sizing and paint pixels.
    ///
    /// The same object is retained as both the layout resolver and the
    /// paint-time pixel source. This matches the intended cache lifetime across
    /// layout and downstream page-fragment painting.
    pub fn replaced_resource_provider<R>(mut self, provider: &'a R) -> Self
    where
        R: ReplacedResolver + ImagePixelSource + Send + Sync + 'a,
    {
        self.reset_resource_state();
        self.resolver = Some(provider);
        self.image_pixel_source = Some(provider);
        self
    }

    /// Set the intrinsic-size resolver when paint pixels are supplied elsewhere.
    pub fn replaced_resolver<R>(mut self, resolver: &'a R) -> Self
    where
        R: ReplacedResolver + Send + Sync + 'a,
    {
        self.resolver = Some(resolver);
        self
    }

    /// Set a separate paint-time image source.
    pub fn image_pixel_source<I>(mut self, source: &'a I) -> Self
    where
        I: ImagePixelSource + Send + Sync + 'a,
    {
        self.reset_resource_state();
        self.image_pixel_source = Some(source);
        self
    }

    /// The font layer given with [`fonts`](Self::fonts); `None` when text is
    /// laid out with the installed fonts.
    pub fn font_collection_ref(&self) -> Option<&shodo::font::FontCollection> {
        self.font_collection.as_ref()
    }

    /// The combined configured image source and CSS background cache for painting.
    ///
    /// This source is always available because it also exposes the shared
    /// CSS background cache, including data URLs preloaded without a network
    /// provider.
    pub fn image_pixel_source_ref(&self) -> Option<&dyn ImagePixelSource> {
        Some(self as &dyn ImagePixelSource)
    }

    pub(crate) fn extra_stylesheets(&self) -> Vec<&str> {
        self.extra_stylesheets.iter().map(String::as_str).collect()
    }

    pub(crate) fn parse_options<'b>(
        &'b self,
        extra_stylesheets: &'b [&'b str],
        network: Option<&'b dyn NetworkProvider>,
    ) -> ParseOptions<'b> {
        ParseOptions {
            extra_stylesheets,
            network,
            base_url: self.base_url.clone(),
        }
    }

    pub(crate) fn parse_limits(&self) -> RenderLimits {
        self.render_limits.clone()
    }

    pub(crate) fn fallback_base_url(&self) -> Option<&Url> {
        self.base_url.as_ref()
    }

    pub(crate) fn policy(&self) -> Option<&dyn ResourcePolicy> {
        self.policy
    }

    pub(crate) fn resolver(&self) -> Option<&(dyn ReplacedResolver + Send + Sync)> {
        self.resolver
    }

    pub(crate) fn raw_image_pixel_source(&self) -> Option<&(dyn ImagePixelSource + Send + Sync)> {
        self.image_pixel_source
    }

    pub(crate) fn network_adapter(&self) -> Option<ResourceNetworkProvider<'_>> {
        self.network.map(|inner| ResourceNetworkProvider {
            inner,
            policy: self.policy,
            limits: self.resource_limits,
            budget: Arc::clone(&self.budget),
        })
    }

    fn acquire_decode_permit(&self) -> ProcessDecodeGuard {
        self.process_budget.acquire()
    }

    fn cache_image_source(&self, url: Url, source: CachedBackgroundSource) {
        self.image_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert_source(url, source);
    }

    fn cache_image_raster(
        &self,
        url: Url,
        source: CachedBackgroundSource,
        size: ImageRasterSize,
        image: Arc<DecodedImage>,
    ) -> Result<(), (u64, u64)> {
        self.image_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert_raster(url, source, size, image)
    }

    fn image_cache_effective_remaining(&self, url: &Url) -> u64 {
        let (per_available, prior) = {
            let cache = self
                .image_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (cache.remaining_for(url), cache.prior_bytes(url))
        };
        per_available.min(self.process_budget.available_for(prior))
    }

    fn cached_background_image(&self, url: &Url) -> Option<CachedBackgroundImage> {
        self.image_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .get(url)
            .cloned()
    }

    fn image_policy_limit(&self) -> Option<u64> {
        self.policy
            .and_then(|policy| policy.max_decoded_bytes(ResourceKind::Image))
    }

    fn decode_deadline(&self) -> Option<std::time::Duration> {
        self.policy
            .map(|policy| policy.decode_timeout(ResourceKind::Image))
    }

    fn join_background_inflight(&self, url: &Url) -> (Arc<BackgroundInflight>, bool) {
        let mut map = self
            .background_inflight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = map.get(url) {
            return (Arc::clone(entry), false);
        }
        let entry = Arc::new(BackgroundInflight::new());
        map.insert(url.clone(), Arc::clone(&entry));
        (entry, true)
    }

    fn finish_background_inflight(
        &self,
        url: &Url,
        entry: &Arc<BackgroundInflight>,
        warning: Option<RenderWarning>,
    ) {
        entry.finish(warning);
        self.background_inflight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(url);
    }

    fn decode_timed_out(&self, elapsed: std::time::Duration) -> bool {
        self.decode_deadline()
            .is_some_and(|timeout| elapsed >= timeout)
    }

    fn abort_warning(&self, url: &Url) -> RenderWarning {
        RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                url: Some(redacted_url(url)),
            },
            node_id: None,
            details: "CSS background image decode was aborted; the image was skipped".into(),
        }
    }

    fn timeout_warning(&self, url: &Url) -> RenderWarning {
        let violation = PolicyViolation {
            kind: ResourceKind::Image,
            url: url.clone(),
            violation_type: ViolationType::Timeout,
            details: "CSS background image decode exceeded its time budget".to_owned(),
        };
        RenderWarning {
            kind: WarningKind::PolicyWarning {
                violation: sanitize_policy_violation(violation),
            },
            node_id: None,
            details: "CSS background image decode timed out; the image was skipped".into(),
        }
    }

    /// Preload the element and page-context background images of one full
    /// cascade, in that order.
    #[cfg(test)]
    pub(crate) fn preload_background_images(
        &self,
        cascade: &raikiri_style::CascadeResult,
        warnings: &SharedRenderWarnings,
        seen: &mut std::collections::HashSet<Url>,
        attempts: &mut usize,
        signal: Option<&AbortSignal>,
    ) {
        self.preload_element_background_images(
            &cascade.computed,
            None,
            warnings,
            seen,
            attempts,
            signal,
        );
        self.preload_page_context_background_images(
            &cascade.page,
            None,
            warnings,
            seen,
            attempts,
            signal,
        );
    }

    /// Preload the CSS background images of rendered elements.
    ///
    /// Element computed values do not depend on the page query, so a paged
    /// caller scans them once and then calls
    /// [`Self::preload_page_context_background_images`] per page. `seen` and
    /// `attempts` are shared across those calls so deduplication and the
    /// request limit span the whole document.
    pub(crate) fn preload_element_background_images(
        &self,
        computed: &[ComputedValues],
        base_url: Option<&Url>,
        warnings: &SharedRenderWarnings,
        seen: &mut std::collections::HashSet<Url>,
        attempts: &mut usize,
        signal: Option<&AbortSignal>,
    ) {
        let raw_urls = computed
            .iter()
            .filter(|computed| {
                !matches!(
                    computed.display,
                    DisplayValue::None | DisplayValue::Contents
                ) && computed.visibility == Visibility::Visible
                    && computed.opacity > 0.0
            })
            .filter_map(|computed| match &computed.background_image {
                BackgroundImage::Url(raw_url) => Some(raw_url.as_str()),
                _ => None,
            });
        self.preload_background_urls(raw_urls, base_url, warnings, seen, attempts, signal);
    }

    /// Fetch image markers before layout because their dimensions affect lines.
    pub(crate) fn preload_list_marker_images(
        &self,
        cascade: &raikiri_style::CascadeResult,
        base_url: Option<&Url>,
        warnings: &SharedRenderWarnings,
        seen: &mut std::collections::HashSet<Url>,
        attempts: &mut usize,
        signal: Option<&AbortSignal>,
    ) {
        let urls: Vec<String> = cascade
            .computed
            .iter()
            .enumerate()
            .filter(|(element, _)| {
                raikiri_dom::generated_content::marker_is_enabled(cascade, *element)
                    && cascade
                        .pseudo
                        .get(&(
                            raikiri_style::StyleNodeId::new(*element as u64),
                            raikiri_style::PseudoElem::Marker,
                        ))
                        .is_none_or(|marker| marker.content.is_empty())
            })
            .filter_map(|(_, cv)| match &cv.list_style_image {
                BackgroundImage::Url(raw) => Url::parse(raw)
                    .ok()
                    .or_else(|| base_url.and_then(|base| base.join(raw).ok()))
                    .map(|url| url.to_string()),
                _ => None,
            })
            .collect();
        self.preload_background_urls(
            urls.iter().map(String::as_str),
            base_url,
            warnings,
            seen,
            attempts,
            signal,
        );
    }

    /// Preload the page-context and page-margin-box background images of one
    /// page.
    pub(crate) fn preload_page_context_background_images(
        &self,
        page: &PageCascadeResult,
        base_url: Option<&Url>,
        warnings: &SharedRenderWarnings,
        seen: &mut std::collections::HashSet<Url>,
        attempts: &mut usize,
        signal: Option<&AbortSignal>,
    ) {
        let mut raw_urls: Vec<&str> = Vec::new();
        if let Some(PropertyValue::BackgroundImage(BackgroundImage::Url(raw_url))) =
            page.declarations().get(&PropertyKey::BackgroundImage)
        {
            raw_urls.push(raw_url);
        }
        let margin_box_rules = page.margin_boxes();
        let mut seen_slots = Vec::new();
        for rule in margin_box_rules {
            if seen_slots.contains(&rule.slot) {
                continue;
            }
            seen_slots.push(rule.slot);
            let winning_background = margin_box_rules
                .iter()
                .filter(|candidate| candidate.slot == rule.slot)
                .flat_map(|candidate| candidate.declarations.iter())
                .filter(|declaration| declaration.value().key() == PropertyKey::BackgroundImage)
                .next_back();
            if let Some(PropertyValue::BackgroundImage(BackgroundImage::Url(raw_url))) =
                winning_background.map(|declaration| declaration.value())
            {
                raw_urls.push(raw_url);
            }
        }
        self.preload_background_urls(raw_urls, base_url, warnings, seen, attempts, signal);
    }

    /// Fetch and decode CSS background images ahead of paint.
    ///
    /// Computed `url()` values are kept as authored, so a relative URL is
    /// resolved here against the document base URL, as list-marker images
    /// are. Paint looks the image up by the same absolute URL.
    fn preload_background_urls<'u>(
        &self,
        raw_urls: impl IntoIterator<Item = &'u str>,
        base_url: Option<&Url>,
        warnings: &SharedRenderWarnings,
        seen: &mut std::collections::HashSet<Url>,
        attempts: &mut usize,
        signal: Option<&AbortSignal>,
    ) {
        for raw_url in raw_urls {
            if signal.is_some_and(|signal| signal.is_aborted()) {
                break;
            }
            // `url()` and `url(#id)` name no external image; joining them
            // against the base would turn them into the document itself.
            let trimmed = raw_url.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some(original_url) = Url::parse(raw_url)
                .ok()
                .or_else(|| base_url.and_then(|base| base.join(raw_url).ok()))
            else {
                continue;
            };
            if original_url.cannot_be_a_base() && original_url.scheme() != "data" {
                continue;
            }
            let url = image_url_without_fragment(&original_url);
            if !seen.insert(url.clone()) {
                continue;
            }
            if let Some(policy) = self.policy {
                let violation_type =
                    if !policy.is_scheme_allowed(original_url.scheme(), ResourceKind::Image) {
                        Some(ViolationType::SchemeNotAllowed)
                    } else if !policy
                        .is_host_allowed(original_url.host_str().unwrap_or(""), ResourceKind::Image)
                    {
                        Some(ViolationType::HostNotAllowed)
                    } else {
                        None
                    };
                if let Some(violation_type) = violation_type {
                    let violation = PolicyViolation {
                        kind: ResourceKind::Image,
                        url: original_url.clone(),
                        violation_type,
                        details: "CSS background image URL denied by resource policy".to_owned(),
                    };
                    push_resource_warning(
                        warnings,
                        RenderWarning {
                            kind: WarningKind::PolicyWarning {
                                violation: sanitize_policy_violation(violation),
                            },
                            node_id: None,
                            details: "CSS background image was denied by the resource policy"
                                .into(),
                        },
                    );
                    continue;
                }
            }
            if self
                .image_pixel_source
                .is_some_and(|source| source.intrinsic_size(&original_url).is_some())
            {
                continue;
            }
            if self.cached_background_image(&url).is_some() {
                continue;
            }
            if *attempts >= MAX_BACKGROUND_IMAGE_ATTEMPTS {
                if *attempts == MAX_BACKGROUND_IMAGE_ATTEMPTS {
                    push_resource_warning(
                        warnings,
                        RenderWarning {
                            kind: WarningKind::ResourceLimitExceeded {
                                kind: ResourceKind::Image,
                                limit: MAX_BACKGROUND_IMAGE_ATTEMPTS as u64,
                                actual: (*attempts + 1) as u64,
                            },
                            node_id: None,
                            details: "CSS background image request limit was reached".into(),
                        },
                    );
                    *attempts += 1;
                }
                continue;
            }
            *attempts += 1;
            self.fetch_background_image(url, warnings, signal);
        }
    }

    fn fetch_background_image(
        &self,
        url: Url,
        warnings: &SharedRenderWarnings,
        signal: Option<&AbortSignal>,
    ) {
        if self.cached_background_image(&url).is_some() {
            return;
        }
        if url.scheme() == "data" {
            let Some((bytes, content_type)) = self.decode_data_background_image(&url, warnings)
            else {
                return;
            };
            let _ = self.decode_and_cache_bytes(url, bytes, Some(content_type), warnings, signal);
            return;
        }
        let (entry, is_owner) = self.join_background_inflight(&url);
        if !is_owner {
            if let Some(warning) = entry.wait_for_done() {
                push_resource_warning(warnings, warning);
            }
            return;
        }
        let warning = self.fetch_and_decode_network_owned(url.clone(), warnings, signal);
        self.finish_background_inflight(&url, &entry, warning);
    }

    fn fetch_and_decode_network_owned(
        &self,
        url: Url,
        warnings: &SharedRenderWarnings,
        signal: Option<&AbortSignal>,
    ) -> Option<RenderWarning> {
        if signal.is_some_and(|signal| signal.is_aborted()) {
            let warning = self.abort_warning(&url);
            push_resource_warning(warnings, warning.clone());
            return Some(warning);
        }
        let Some(network) = self.network_adapter() else {
            let warning = RenderWarning {
                kind: WarningKind::NetworkFallback {
                    url: redacted_url(&url),
                },
                node_id: None,
                details:
                    "CSS background image was skipped because no network provider is configured"
                        .into(),
            };
            push_resource_warning(warnings, warning.clone());
            return Some(warning);
        };
        let fetched = match network.fetch(get_request(url.clone(), ResourceKind::Image, signal)) {
            Ok(fetched) => fetched,
            Err(NetworkError::PolicyViolation(violation)) => {
                let kind = match &violation.violation_type {
                    ViolationType::FetchTooLarge { limit, actual } => {
                        WarningKind::ResourceLimitExceeded {
                            kind: ResourceKind::Image,
                            limit: *limit,
                            actual: *actual,
                        }
                    }
                    _ => WarningKind::PolicyWarning {
                        violation: sanitize_policy_violation(*violation),
                    },
                };
                let warning = RenderWarning {
                    kind,
                    node_id: None,
                    details: "CSS background image was denied by the resource policy".into(),
                };
                push_resource_warning(warnings, warning.clone());
                return Some(warning);
            }
            Err(NetworkError::Aborted) => {
                let warning = RenderWarning {
                    kind: WarningKind::NetworkFallback {
                        url: redacted_url(&url),
                    },
                    node_id: None,
                    details: "CSS background image fetch was aborted; the image was skipped".into(),
                };
                push_resource_warning(warnings, warning.clone());
                return Some(warning);
            }
            Err(_) => {
                let warning = RenderWarning {
                    kind: WarningKind::NetworkFallback {
                        url: redacted_url(&url),
                    },
                    node_id: None,
                    details: "CSS background image fetch failed; the image was skipped".into(),
                };
                push_resource_warning(warnings, warning.clone());
                return Some(warning);
            }
        };
        self.decode_and_cache_bytes(
            url,
            fetched.bytes.to_vec(),
            fetched.content_type,
            warnings,
            signal,
        )
    }

    fn decode_and_cache_bytes(
        &self,
        url: Url,
        bytes: Vec<u8>,
        content_type: Option<String>,
        warnings: &SharedRenderWarnings,
        signal: Option<&AbortSignal>,
    ) -> Option<RenderWarning> {
        if signal.is_some_and(|signal| signal.is_aborted()) {
            let warning = self.abort_warning(&url);
            push_resource_warning(warnings, warning.clone());
            return Some(warning);
        }
        let is_svg = content_type.as_deref().is_some_and(|content_type| {
            content_type
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("image/svg+xml"))
        }) || url.path().to_ascii_lowercase().ends_with(".svg");
        let start = Instant::now();
        let _permit = self.acquire_decode_permit();
        if is_svg {
            if bytes.len() > MAX_SVG_IMAGE_BYTES {
                let warning = RenderWarning {
                    kind: WarningKind::ResourceLimitExceeded {
                        kind: ResourceKind::Image,
                        limit: MAX_SVG_IMAGE_BYTES as u64,
                        actual: bytes.len() as u64,
                    },
                    node_id: None,
                    details: "SVG background image exceeded its input byte limit".into(),
                };
                push_resource_warning(warnings, warning.clone());
                return Some(warning);
            }
            match SvgDocument::parse(&bytes) {
                Ok(document) => {
                    if self.decode_timed_out(start.elapsed()) {
                        let warning = self.timeout_warning(&url);
                        push_resource_warning(warnings, warning.clone());
                        return Some(warning);
                    }
                    self.cache_image_source(url, CachedBackgroundSource::Svg(Arc::new(document)));
                    return None;
                }
                Err(_) => {
                    let warning = RenderWarning {
                        kind: WarningKind::ResourceFallback {
                            kind: ResourceKind::Image,
                            url: Some(redacted_url(&url)),
                        },
                        node_id: None,
                        details: "CSS background SVG could not be parsed; the image was skipped"
                            .into(),
                    };
                    push_resource_warning(warnings, warning.clone());
                    return Some(warning);
                }
            }
        }

        let effective = self.image_cache_effective_remaining(&url);
        let output_limit = self.image_policy_limit().unwrap_or(u64::MAX).min(effective);
        match decode_background_raster(&bytes, output_limit) {
            Ok(image) => {
                if self.decode_timed_out(start.elapsed()) {
                    let warning = self.timeout_warning(&url);
                    push_resource_warning(warnings, warning.clone());
                    return Some(warning);
                }
                let width = image.width as f32;
                let height = image.height as f32;
                let image = Arc::new(image);
                if let Err((limit, actual)) = self.cache_image_raster(
                    url.clone(),
                    CachedBackgroundSource::Raster(image.clone()),
                    ImageRasterSize { width, height },
                    image,
                ) {
                    let warning = RenderWarning {
                        kind: WarningKind::ResourceLimitExceeded {
                            kind: ResourceKind::Image,
                            limit,
                            actual,
                        },
                        node_id: None,
                        details: "decoded CSS background image exceeded its cache byte limit"
                            .into(),
                    };
                    push_resource_warning(warnings, warning.clone());
                    return Some(warning);
                }
                None
            }
            Err(BackgroundImageDecodeError::DecodedTooLarge { limit, actual }) => {
                let warning = RenderWarning {
                    kind: WarningKind::ResourceLimitExceeded {
                        kind: ResourceKind::Image,
                        limit,
                        actual,
                    },
                    node_id: None,
                    details:
                        "decoded CSS background image exceeded its per-image or cache byte limit"
                            .into(),
                };
                push_resource_warning(warnings, warning.clone());
                Some(warning)
            }
            Err(BackgroundImageDecodeError::Decode) => {
                if self.decode_timed_out(start.elapsed()) {
                    let warning = self.timeout_warning(&url);
                    push_resource_warning(warnings, warning.clone());
                    return Some(warning);
                }
                let warning = RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: ResourceKind::Image,
                        url: Some(redacted_url(&url)),
                    },
                    node_id: None,
                    details: "CSS background image could not be decoded; the image was skipped"
                        .into(),
                };
                push_resource_warning(warnings, warning.clone());
                Some(warning)
            }
        }
    }

    fn decode_data_background_image(
        &self,
        url: &Url,
        warnings: &SharedRenderWarnings,
    ) -> Option<(Vec<u8>, String)> {
        let violation_type = self.policy.and_then(|policy| {
            if !policy.is_scheme_allowed(url.scheme(), ResourceKind::Image) {
                Some(ViolationType::SchemeNotAllowed)
            } else if !policy.is_host_allowed("", ResourceKind::Image) {
                Some(ViolationType::HostNotAllowed)
            } else {
                None
            }
        });
        if let Some(violation_type) = violation_type {
            let violation = PolicyViolation {
                kind: ResourceKind::Image,
                url: url.clone(),
                violation_type,
                details: "inline image URL denied by resource policy".to_owned(),
            };
            push_resource_warning(
                warnings,
                RenderWarning {
                    kind: WarningKind::PolicyWarning {
                        violation: sanitize_policy_violation(violation),
                    },
                    node_id: None,
                    details: "CSS background image was denied by the resource policy".into(),
                },
            );
            return None;
        }

        let raw = url.as_str();
        let Some(metadata) = raw
            .strip_prefix("data:")
            .and_then(|raw| raw.split_once(',').map(|(metadata, _)| metadata))
        else {
            push_image_fallback_warning(
                warnings,
                url,
                "inline CSS background image data URL was malformed",
            );
            return None;
        };
        let mime = metadata
            .split(';')
            .next()
            .filter(|mime| !mime.is_empty())
            .unwrap_or("text/plain")
            .to_owned();
        let data_url = match data_url::DataUrl::process(raw) {
            Ok(data_url) => data_url,
            Err(_) => {
                push_image_fallback_warning(
                    warnings,
                    url,
                    "inline CSS background image data URL was invalid",
                );
                return None;
            }
        };
        let (bytes, _) = match data_url.decode_to_vec() {
            Ok(decoded) => decoded,
            Err(_) => {
                push_image_fallback_warning(
                    warnings,
                    url,
                    "inline CSS background image data URL payload was invalid",
                );
                return None;
            }
        };
        let actual = bytes.len() as u64;
        let policy_limit = self
            .policy
            .and_then(|policy| policy.max_fetch_bytes(ResourceKind::Image));
        let limit = match (self.resource_limits.max_resource_bytes, policy_limit) {
            (Some(local), Some(policy)) => Some(local.min(policy)),
            (Some(local), None) | (None, Some(local)) => Some(local),
            (None, None) => None,
        };
        if let Some(limit) = limit
            && actual > limit
        {
            push_image_limit_warning(
                warnings,
                limit,
                actual,
                "inline CSS background image exceeded its response byte limit",
            );
            return None;
        }
        let mime_denied = self.policy.is_some_and(|policy| {
            let allowed_mimes = policy.allowed_mime_types(ResourceKind::Image);
            !allowed_mimes.is_empty()
                && !allowed_mimes
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(&mime))
        });
        if mime_denied {
            let violation = PolicyViolation {
                kind: ResourceKind::Image,
                url: url.clone(),
                violation_type: ViolationType::MimeNotAllowed { mime: mime.clone() },
                details: "inline image MIME type denied by resource policy".to_owned(),
            };
            push_resource_warning(
                warnings,
                RenderWarning {
                    kind: WarningKind::PolicyWarning {
                        violation: sanitize_policy_violation(violation),
                    },
                    node_id: None,
                    details: "CSS background image MIME type was denied by the resource policy"
                        .into(),
                },
            );
            return None;
        }

        let mut budget = self
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let total = budget.saturating_add(actual);
        if let Some(limit) = self.resource_limits.max_aggregate_resource_bytes
            && total > limit
        {
            push_image_limit_warning(
                warnings,
                limit,
                total,
                "aggregate image responses exceeded their byte limit",
            );
            return None;
        }
        *budget = total;
        Some((bytes, mime))
    }
}

impl ImagePixelSource for RenderResources<'_> {
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        if !self.image_url_allowed(url) {
            return None;
        }
        let canonical = image_url_without_fragment(url);
        if let Some(cached) = self.cached_background_image(&canonical) {
            match cached.source {
                CachedBackgroundSource::Raster(image) => return Some(image),
                CachedBackgroundSource::Svg(document) => {
                    let size = default_background_raster_size(svg_intrinsic_size(&document));
                    return self.get_decoded_at_size(url, size, None);
                }
            }
        }
        let image = self.image_pixel_source?.get_decoded(url)?;
        self.image_within_policy(image)
    }

    fn intrinsic_size(&self, url: &Url) -> Option<ImageIntrinsicSize> {
        if !self.image_url_allowed(url) {
            return None;
        }
        let canonical = image_url_without_fragment(url);
        if let Some(cached) = self.cached_background_image(&canonical) {
            return Some(match cached.source {
                CachedBackgroundSource::Raster(image) => image_intrinsic_size(&image),
                CachedBackgroundSource::Svg(document) => svg_intrinsic_size(&document),
            });
        }
        self.image_pixel_source?.intrinsic_size(url)
    }

    fn decoded_byte_len(&self, url: &Url) -> Option<u64> {
        if !self.image_url_allowed(url) {
            return None;
        }
        let canonical = image_url_without_fragment(url);
        if let Some(cached) = self.cached_background_image(&canonical) {
            return match cached.source {
                CachedBackgroundSource::Raster(image) => Some(image.rgba.len() as u64),
                CachedBackgroundSource::Svg(_) => {
                    cached.raster.map(|(_, image)| image.rgba.len() as u64)
                }
            };
        }
        self.image_pixel_source?.decoded_byte_len(url)
    }

    fn get_decoded_at_size(
        &self,
        url: &Url,
        size: ImageRasterSize,
        max_output_bytes: Option<u64>,
    ) -> Option<Arc<DecodedImage>> {
        if !self.image_url_allowed(url) {
            return None;
        }
        let policy_limit = self
            .policy
            .and_then(|policy| policy.max_decoded_bytes(ResourceKind::Image));
        let effective_limit = match (max_output_bytes, policy_limit) {
            (Some(requested), Some(policy)) => Some(requested.min(policy)),
            (Some(requested), None) => Some(requested),
            (None, limit) => limit,
        };
        let canonical = image_url_without_fragment(url);
        if let Some(cached) = self.cached_background_image(&canonical) {
            match cached.source {
                CachedBackgroundSource::Raster(image) => {
                    if effective_limit.is_some_and(|limit| image.rgba.len() as u64 > limit) {
                        return None;
                    }
                    return Some(image);
                }
                CachedBackgroundSource::Svg(_) => {
                    if let Some((cached_size, image)) = cached.raster
                        && cached_size == size
                        && effective_limit.is_none_or(|limit| image.rgba.len() as u64 <= limit)
                    {
                        return Some(image);
                    }
                    let start = Instant::now();
                    let _permit = self.acquire_decode_permit();
                    let cached = self.cached_background_image(&canonical)?;
                    if let Some((cached_size, image)) = cached.raster
                        && cached_size == size
                        && effective_limit.is_none_or(|limit| image.rgba.len() as u64 <= limit)
                    {
                        return Some(image);
                    }
                    let document = match cached.source {
                        CachedBackgroundSource::Svg(document) => document,
                        CachedBackgroundSource::Raster(image) => {
                            if effective_limit.is_some_and(|limit| image.rgba.len() as u64 > limit)
                            {
                                return None;
                            }
                            return Some(image);
                        }
                    };
                    let cache_limit = self.image_cache_effective_remaining(&canonical);
                    let limit = effective_limit.unwrap_or(u64::MAX).min(cache_limit);
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
                    if self.decode_timed_out(start.elapsed()) {
                        return None;
                    }
                    let image = Arc::new(image);
                    self.cache_image_raster(
                        canonical,
                        CachedBackgroundSource::Svg(document),
                        size,
                        image.clone(),
                    )
                    .ok()?;
                    return Some(image);
                }
            }
        }
        let image = self
            .image_pixel_source?
            .get_decoded_at_size(url, size, effective_limit)?;
        if effective_limit.is_some_and(|limit| image.rgba.len() as u64 > limit) {
            return None;
        }
        Some(image)
    }
}

impl RenderResources<'_> {
    fn image_url_allowed(&self, url: &Url) -> bool {
        self.policy.is_none_or(|policy| {
            policy.is_scheme_allowed(url.scheme(), ResourceKind::Image)
                && policy.is_host_allowed(url.host_str().unwrap_or(""), ResourceKind::Image)
        })
    }

    fn image_within_policy(&self, image: Arc<DecodedImage>) -> Option<Arc<DecodedImage>> {
        self.image_policy_limit()
            .is_none_or(|limit| image.rgba.len() as u64 <= limit)
            .then_some(image)
    }
}

#[derive(Debug)]
enum BackgroundImageDecodeError {
    Decode,
    DecodedTooLarge { limit: u64, actual: u64 },
}

fn decode_background_raster(
    bytes: &[u8],
    max_decoded_bytes: u64,
) -> Result<DecodedImage, BackgroundImageDecodeError> {
    const DECODER_ALLOCATION_HEADROOM: u64 = 16 * 1024 * 1024;
    const DECODER_ALLOCATION_HARD_LIMIT: u64 = 512 * 1024 * 1024;

    let mut limits = Limits::default();
    limits.max_alloc = Some(
        max_decoded_bytes
            .saturating_mul(2)
            .saturating_add(DECODER_ALLOCATION_HEADROOM)
            .min(DECODER_ALLOCATION_HARD_LIMIT),
    );
    let mut reader = ImageReader::new(Cursor::new(bytes));
    reader.limits(limits);
    let decoder = reader
        .with_guessed_format()
        .map_err(|_| BackgroundImageDecodeError::Decode)?
        .into_decoder()
        .map_err(|_| BackgroundImageDecodeError::Decode)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(BackgroundImageDecodeError::Decode);
    }
    let expected_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(BackgroundImageDecodeError::Decode)?;
    if expected_bytes > max_decoded_bytes {
        return Err(BackgroundImageDecodeError::DecodedTooLarge {
            limit: max_decoded_bytes,
            actual: expected_bytes,
        });
    }

    let rgba = DynamicImage::from_decoder(decoder)
        .map_err(|_| BackgroundImageDecodeError::Decode)?
        .into_rgba8();
    let pixels = rgba.into_raw();
    let actual_bytes = u64::try_from(pixels.len()).unwrap_or(u64::MAX);
    if actual_bytes != expected_bytes {
        return Err(BackgroundImageDecodeError::Decode);
    }
    if actual_bytes > max_decoded_bytes {
        return Err(BackgroundImageDecodeError::DecodedTooLarge {
            limit: max_decoded_bytes,
            actual: actual_bytes,
        });
    }
    Ok(DecodedImage {
        width,
        height,
        rgba: pixels,
    })
}

fn image_url_without_fragment(url: &Url) -> Url {
    let mut normalized = url.clone();
    normalized.set_fragment(None);
    normalized
}

fn image_intrinsic_size(image: &DecodedImage) -> ImageIntrinsicSize {
    let width = image.width as f32;
    let height = image.height as f32;
    ImageIntrinsicSize {
        width: Some(width),
        height: Some(height),
        aspect_ratio: (height > 0.0).then_some(width / height),
    }
}

fn svg_intrinsic_size(document: &SvgDocument) -> ImageIntrinsicSize {
    let intrinsic = document.intrinsic_size();
    ImageIntrinsicSize {
        width: intrinsic.width,
        height: intrinsic.height,
        aspect_ratio: intrinsic.aspect_ratio,
    }
}

fn default_background_raster_size(intrinsic: ImageIntrinsicSize) -> ImageRasterSize {
    const DEFAULT_WIDTH: f32 = 300.0;
    const DEFAULT_HEIGHT: f32 = 150.0;
    match (intrinsic.width, intrinsic.height, intrinsic.aspect_ratio) {
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

fn push_image_limit_warning(
    warnings: &SharedRenderWarnings,
    limit: u64,
    actual: u64,
    details: &str,
) {
    push_resource_warning(
        warnings,
        RenderWarning {
            kind: WarningKind::ResourceLimitExceeded {
                kind: ResourceKind::Image,
                limit,
                actual,
            },
            node_id: None,
            details: details.to_owned(),
        },
    );
}

fn push_image_fallback_warning(warnings: &SharedRenderWarnings, url: &Url, details: &str) {
    push_resource_warning(
        warnings,
        RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                url: Some(redacted_url(url)),
            },
            node_id: None,
            details: details.to_owned(),
        },
    );
}

/// Network provider adapter shared by stylesheet imports and font loads.
pub(crate) struct ResourceNetworkProvider<'a> {
    inner: &'a dyn NetworkProvider,
    policy: Option<&'a dyn ResourcePolicy>,
    limits: ResourceLimits,
    budget: Arc<Mutex<u64>>,
}

impl ResourceNetworkProvider<'_> {
    /// Builds a `PolicyViolation` naming `url` — the URL the check that
    /// failed actually inspected, which after a redirect hop is not
    /// necessarily `request.url` — rather than always reporting the
    /// original request URL regardless of which URL was actually denied.
    fn violation(
        request: &Request,
        url: &Url,
        violation_type: ViolationType,
        details: impl Into<String>,
    ) -> NetworkError {
        NetworkError::PolicyViolation(Box::new(PolicyViolation {
            kind: request.kind,
            url: url.clone(),
            violation_type,
            details: details.into(),
        }))
    }

    fn check_url_policy(&self, request: &Request, url: &Url) -> Result<(), NetworkError> {
        let Some(policy) = self.policy else {
            return Ok(());
        };
        if !policy.is_scheme_allowed(url.scheme(), request.kind) {
            return Err(Self::violation(
                request,
                url,
                ViolationType::SchemeNotAllowed,
                "resource URL scheme is denied by policy",
            ));
        }
        let host = url.host_str().unwrap_or("");
        if !policy.is_host_allowed(host, request.kind) {
            return Err(Self::violation(
                request,
                url,
                ViolationType::HostNotAllowed,
                "resource URL host is denied by policy",
            ));
        }
        Ok(())
    }

    fn byte_limit(&self, kind: ResourceKind) -> Option<u64> {
        let policy_limit = self.policy.and_then(|policy| policy.max_fetch_bytes(kind));
        match (self.limits.max_resource_bytes, policy_limit) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(limit), None) | (None, Some(limit)) => Some(limit),
            (None, None) => None,
        }
    }
}

impl NetworkProvider for ResourceNetworkProvider<'_> {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.check_url_policy(&request, &request.url)?;

        // Drives `inner.fetch_one_hop` itself rather than delegating to
        // `inner.fetch()`, so every hop's target is checked against policy
        // *before* any request reaches it — not only the initial URL and
        // the final response's URL after the fact, by which point a
        // provider that follows redirects internally (e.g. `ureq`'s
        // default behavior) would have already sent every intermediate
        // request. `hop` here is the real count of redirects actually
        // taken, unlike a fixed `1` passed to every `allow_redirect` call.
        let mut current = request.clone();
        let mut hop = 0u32;
        let fetched = loop {
            match self.inner.fetch_one_hop(current.clone())? {
                FetchOutcome::Body(fetched) => break fetched,
                FetchOutcome::Redirect { location, .. } => {
                    if let Some(policy) = self.policy
                        && (hop >= policy.max_redirect_hops(request.kind)
                            || !policy.allow_redirect(&current.url, &location, hop + 1))
                    {
                        return Err(Self::violation(
                            &request,
                            &location,
                            ViolationType::RedirectDenied,
                            "resource redirect is denied by policy",
                        ));
                    }
                    // This adapter consumes redirects internally, so the
                    // outer NetworkProvider::fetch loop cannot enforce its cap.
                    if self.policy.is_none() && hop >= raikiri_traits::MAX_AUTO_REDIRECT_HOPS {
                        return Err(NetworkError::Other("too many redirects".to_owned()));
                    }
                    hop += 1;
                    self.check_url_policy(&request, &location)?;
                    current.url = location;
                }
                // cov:ignore: `FetchOutcome` is `#[non_exhaustive]`; this
                // crate cannot construct a third variant to exercise this
                // arm from a test, only `Body`/`Redirect` exist today.
                _ => {
                    return Err(NetworkError::Other(
                        "unknown FetchOutcome variant".to_owned(),
                    ));
                }
            }
        };

        let actual = fetched.bytes.len() as u64;
        if let Some(limit) = self.byte_limit(request.kind)
            && actual > limit
        {
            return Err(Self::violation(
                &request,
                &request.url,
                ViolationType::FetchTooLarge { limit, actual },
                "resource response exceeded its byte limit",
            ));
        }

        if let Some(policy) = self.policy {
            let allowed = policy.allowed_mime_types(request.kind);
            if !allowed.is_empty() {
                let content_type = fetched.content_type.as_deref().ok_or_else(|| {
                    Self::violation(
                        &request,
                        &request.url,
                        ViolationType::MimeNotAllowed {
                            mime: "<missing>".to_owned(),
                        },
                        "resource response did not include a required MIME type",
                    )
                })?;
                let mime = content_type
                    .split(';')
                    .next()
                    .unwrap_or(content_type)
                    .trim();
                if !allowed
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(mime))
                {
                    return Err(Self::violation(
                        &request,
                        &request.url,
                        ViolationType::MimeNotAllowed {
                            mime: mime.to_owned(),
                        },
                        "resource MIME type is denied by policy",
                    ));
                }
            }
        }

        let mut used = self
            .budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let total = used.saturating_add(actual);
        if let Some(limit) = self.limits.max_aggregate_resource_bytes
            && total > limit
        {
            return Err(Self::violation(
                &request,
                &request.url,
                ViolationType::FetchTooLarge {
                    limit,
                    actual: total,
                },
                "aggregate resource responses exceeded their byte limit",
            ));
        }
        *used = total;
        Ok(FetchOutcome::Body(fetched))
    }

    fn max_import_depth(&self) -> Option<u32> {
        match (
            self.policy.map(ResourcePolicy::max_import_depth),
            self.inner.max_import_depth(),
        ) {
            (Some(policy), Some(provider)) => Some(policy.min(provider)),
            (Some(limit), None) | (None, Some(limit)) => Some(limit),
            (None, None) => None,
        }
    }
}

/// Build a GET request for one resolved resource URL.
///
/// The caller passes its render abort signal through so a provider that honors
/// [`AbortSignal`] can fail fast instead of
/// buffering a response nobody will use. Providers only check the signal up
/// front; a signal that fires mid-fetch does not interrupt bytes already in
/// flight.
pub(crate) fn get_request(url: Url, kind: ResourceKind, signal: Option<&AbortSignal>) -> Request {
    Request {
        url,
        method: Method::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: signal.cloned(),
        kind,
    }
}

/// Warnings collected while resolving CSS `@font-face` sources.
pub(crate) type SharedRenderWarnings = Arc<Mutex<Vec<RenderWarning>>>;

pub(crate) fn push_resource_warning(warnings: &SharedRenderWarnings, warning: RenderWarning) {
    warnings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(warning);
}

pub(crate) fn redacted_url(url: &Url) -> Url {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted
}

pub(crate) fn sanitize_policy_violation(mut violation: PolicyViolation) -> PolicyViolation {
    violation.url = redacted_url(&violation.url);
    violation.details = "network policy denied the request".to_owned();
    violation
}

/// Adapter that routes font-face URLs through the same policy-checked provider
/// and aggregate byte budget used by stylesheet parsing.
pub(crate) struct NetworkFontFaceLoader<'a> {
    network: Option<&'a dyn NetworkProvider>,
    base_url: Option<&'a Url>,
    warnings: SharedRenderWarnings,
    /// The outcome of every source loaded so far, failures included. One
    /// layout reads each `@font-face` source several times (once per
    /// cascade, and once more for the inline engine's font layer); without
    /// this every read would fetch again, spend the aggregate budget again,
    /// and repeat its warning.
    loaded: Mutex<HashMap<Url, Option<Vec<u8>>>>,
}

impl<'a> NetworkFontFaceLoader<'a> {
    pub(crate) fn new(
        network: Option<&'a dyn NetworkProvider>,
        base_url: Option<&'a Url>,
        warnings: SharedRenderWarnings,
    ) -> Self {
        Self {
            network,
            base_url,
            warnings,
            loaded: Mutex::new(HashMap::new()),
        }
    }
}

impl FontFaceLoader for NetworkFontFaceLoader<'_> {
    fn load(&self, source: &str) -> Option<Vec<u8>> {
        let url = Url::parse(source)
            .ok()
            .or_else(|| self.base_url.and_then(|base| base.join(source).ok()));
        let Some(url) = url else {
            push_resource_warning(
                &self.warnings,
                RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: ResourceKind::Font,
                        url: None,
                    },
                    node_id: None,
                    details: "font-face source could not be resolved against the document base"
                        .into(),
                },
            );
            return None;
        };
        let mut loaded = self
            .loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(outcome) = loaded.get(&url) {
            return outcome.clone();
        }
        let outcome = self.fetch_uncached(&url);
        loaded.insert(url, outcome.clone());
        outcome
    }
}

impl NetworkFontFaceLoader<'_> {
    fn fetch_uncached(&self, url: &Url) -> Option<Vec<u8>> {
        let Some(network) = self.network else {
            push_resource_warning(
                &self.warnings,
                RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: ResourceKind::Font,
                        url: Some(redacted_url(url)),
                    },
                    node_id: None,
                    details: "font-face source skipped because no network provider is configured"
                        .into(),
                },
            );
            return None;
        };

        match network.fetch(get_request(url.clone(), ResourceKind::Font, None)) {
            Ok(fetched) => Some(fetched.bytes.to_vec()),
            Err(NetworkError::PolicyViolation(violation)) => {
                let kind = match &violation.violation_type {
                    ViolationType::FetchTooLarge { limit, actual }
                    | ViolationType::DecodedTooLarge { limit, actual } => {
                        WarningKind::ResourceLimitExceeded {
                            kind: ResourceKind::Font,
                            limit: *limit,
                            actual: *actual,
                        }
                    }
                    _ => WarningKind::PolicyWarning {
                        violation: sanitize_policy_violation((*violation).clone()),
                    },
                };
                push_resource_warning(
                    &self.warnings,
                    RenderWarning {
                        kind,
                        node_id: None,
                        details: "font-face source was denied by the resource policy".into(),
                    },
                );
                None
            }
            Err(_) => {
                push_resource_warning(
                    &self.warnings,
                    RenderWarning {
                        kind: WarningKind::NetworkFallback {
                            url: redacted_url(url),
                        },
                        node_id: None,
                        details: "font-face fetch failed; the family will use fallback fonts"
                            .into(),
                    },
                );
                None
            }
        }
    }
}

/// Parse HTML using the same renderer-neutral resource configuration later
/// accepted by [`crate::LayoutOptions::resources`].
pub fn parse_html_with_resources<R: Read>(
    input: R,
    resources: &RenderResources<'_>,
) -> Result<HtmlDocument, raikiri_traits::RenderError> {
    let network = resources.network_adapter();
    let network_ref = network
        .as_ref()
        .map(|provider| provider as &dyn NetworkProvider);
    let extra_stylesheets = resources.extra_stylesheets();
    let options = resources.parse_options(&extra_stylesheets, network_ref);
    crate::document_parse::parse_html_with_limits(input, &options, resources.parse_limits())
}

#[cfg(test)]
mod tests;
