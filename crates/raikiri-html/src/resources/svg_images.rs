//! Pictures for the `<image>` elements of SVG documents, taken from the
//! image resources of a page.
//!
//! An SVG draws a picture by asking for it, and this source answers from the
//! same code that serves the images of the HTML around it: the URL is
//! resolved against the document's base, checked by the resource policy,
//! fetched through the network provider under its byte limits and the
//! aggregate budget, and decoded under the per-image and cache limits.
//! Nothing here opens a file or a connection on its own.

use std::collections::HashSet;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use raikiri_svg::SvgImageSource;
use raikiri_traits::{
    AbortSignal, DecodedImage, ImagePixelSource, RenderWarning, ResourceKind, WarningKind,
};
use url::Url;

use super::InlineImageLease;
use super::{
    CachedBackgroundSource, MAX_BACKGROUND_IMAGE_ATTEMPTS, RenderResources, SharedRenderWarnings,
    push_resource_warning,
};
use crate::fetch::{resolve_url, url_without_fragment};

mod pending;
use pending::{DataFlight, PendingData};

/// The `<image>` pictures of SVG documents, taken from a [`RenderResources`].
///
/// Pass it to [`raikiri_svg::SvgResources::with_images`]. A picture is a
/// raster image: an SVG document, however it is named, is not drawn by an SVG,
/// which keeps one SVG from leading to another or back to itself. An image
/// the source cannot give is left out of the scene, and [`Self::take_warnings`]
/// says why when the page's checks do: denied by the policy, not fetched, over
/// a limit, or not a picture.
///
/// Every distinct URL is requested at most once, whatever the number of
/// scenes built with the source, and counts toward the same request limit as
/// the CSS background images of the page (256 per source). Requests run one
/// after another; the provider sees the abort signal of
/// [`Self::abort_signal`].
///
/// Create one for each document with [`RenderResources::svg_images`].
/// Data-picture keys, metadata and decoded pixels share the resource set's
/// 128 MiB image cache budget and the 256 MiB process budget. New pictures
/// that cannot fit are skipped with a warning; existing memo hits remain
/// available. Dropping this source releases its reservations, even when a
/// caller keeps a returned image alive outside the cache.
/// Concurrent requests for the same checked data picture share its outcome.
pub struct RenderSvgImages<'r, 'a> {
    resources: &'r RenderResources<'a>,
    base_url: Option<Url>,
    signal: Option<AbortSignal>,
    warnings: SharedRenderWarnings,
    state: Mutex<State>,
}

// Direct comparisons avoid hashing overhead for small candidate sets.
const LINEAR_DATA_CACHE_ENTRIES: usize = 16;

#[derive(Default)]
struct State {
    /// The URLs already requested, without fragment.
    seen: HashSet<Url>,
    /// The requests made, `data:` pictures included.
    attempts: usize,
    /// The `data:` pictures that passed the byte checks, whether they decoded
    /// or not: asking again must not charge the page's budget again.
    data: Vec<DataPicture>,
    data_hash: RandomState,
}

struct DataPicture {
    fingerprint: u64,
    mime: String,
    bytes: Vec<u8>,
    image: Option<Arc<DecodedImage>>,
    pending: Option<Arc<DataFlight>>,
    // Drop owned keys and pixels before returning their cache reservation.
    _pixels_lease: Option<InlineImageLease>,
    _lease: InlineImageLease,
}

// A conservative accounting policy covers initial Vec capacity, later spare
// slots, and the temporary flight's Arc allocation. This is not a claim about
// the standard library's growth policy or allocator overhead.
const DATA_ENTRY_METADATA_BYTES: u64 = (4 * std::mem::size_of::<DataPicture>()
    + std::mem::size_of::<DataFlight>()
    + 2 * std::mem::size_of::<usize>()) as u64;

enum DataLookup {
    Ready(Option<Arc<DecodedImage>>),
    Pending(Arc<DataFlight>),
}

impl DataLookup {
    fn resolve(self) -> Option<Arc<DecodedImage>> {
        match self {
            Self::Ready(image) => image,
            Self::Pending(flight) => flight.wait(),
        }
    }
}

impl DataPicture {
    fn lookup(&self) -> DataLookup {
        match &self.pending {
            Some(flight) => DataLookup::Pending(Arc::clone(flight)),
            None => DataLookup::Ready(self.image.clone()),
        }
    }
    fn matches(&self, mime: &str, bytes: &[u8]) -> bool {
        if self.bytes.len() != bytes.len() || self.mime != mime {
            return false;
        }
        #[cfg(test)]
        tests::record_comparison(bytes.len());
        self.bytes == bytes
    }
}

impl State {
    fn cached_data(&self, mime: &str, bytes: &[u8]) -> Option<&DataPicture> {
        if self.data.len() <= LINEAR_DATA_CACHE_ENTRIES {
            return self.data.iter().find(|known| known.matches(mime, bytes));
        }
        // Preserve first-entry hits without hashing the encoded payload.
        let first = self.data.first()?;
        if first.matches(mime, bytes) {
            return Some(first);
        }
        let mut fingerprint = None;
        self.data.iter().skip(1).find(|known| {
            // Avoid hashing a large impossible candidate, and avoid comparing
            // a long MIME string against every entry before filtering.
            if known.bytes.len() != bytes.len() || known.mime.len() != mime.len() {
                return false;
            }
            let requested = *fingerprint.get_or_insert_with(|| self.fingerprint(mime, bytes));
            known.fingerprint == requested && known.matches(mime, bytes)
        })
    }

    fn fingerprint(&self, mime: &str, bytes: &[u8]) -> u64 {
        #[cfg(test)]
        tests::record_hash(mime.len() + bytes.len());
        self.data_hash.hash_one((mime, bytes))
    }

    fn remember_data(
        &mut self,
        mime: &str,
        bytes: &[u8],
        image: Option<Arc<DecodedImage>>,
        lease: InlineImageLease,
    ) -> usize {
        let index = self.data.len();
        let fingerprint = self.fingerprint(mime, bytes);
        self.data.push(DataPicture {
            fingerprint,
            mime: mime.to_owned(),
            bytes: bytes.to_vec(),
            image,
            pending: None,
            _pixels_lease: None,
            _lease: lease,
        });
        index
    }
}

impl<'a> RenderResources<'a> {
    /// The source of the `<image>` pictures of an SVG document, taken from
    /// this resource set.
    ///
    /// `base_url` is the base the document's relative URLs resolve against,
    /// which is [`crate::DocumentLayout::base_url`] for the SVG of a page, or
    /// the one set with [`Self::base_url`] when `None`. Without either, only
    /// absolute URLs are requested.
    pub fn svg_images(&self, base_url: Option<Url>) -> RenderSvgImages<'_, 'a> {
        RenderSvgImages {
            resources: self,
            base_url: base_url.or_else(|| self.base_url.clone()),
            signal: None,
            warnings: SharedRenderWarnings::default(),
            state: Mutex::new(State::default()),
        }
    }
}

impl RenderSvgImages<'_, '_> {
    /// Stops the requests that have not been made once `signal` is aborted.
    #[must_use]
    pub fn abort_signal(mut self, signal: AbortSignal) -> Self {
        self.signal = Some(signal);
        self
    }

    /// The warnings of the pictures asked for since the last call.
    pub fn take_warnings(&self) -> Vec<RenderWarning> {
        std::mem::take(&mut *self.warnings.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // A panic in a request leaves the lists consistent: entries are only
        // ever appended whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves the warnings of one request to the source's own, saying whose
    /// they are: the checks behind them are the ones of the CSS images.
    fn keep(&self, request: &SharedRenderWarnings) {
        let made = std::mem::take(&mut *request.lock().unwrap_or_else(PoisonError::into_inner));
        for mut warning in made {
            warning.details = format!("SVG image: {}", warning.details);
            push_resource_warning(&self.warnings, warning);
        }
    }

    fn warn(&self, kind: WarningKind, details: &str) {
        push_resource_warning(&self.warnings, RenderWarning::new(kind, details.into()));
    }

    /// The picture of an `<image>` that the request left in the page's
    /// cache, or that the page's own pixel source holds.
    fn cached(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        match self
            .resources
            .cached_background_image(&url_without_fragment(url))
        {
            Some(cached) => match cached.source {
                CachedBackgroundSource::Raster(image) => Some(image),
                CachedBackgroundSource::Svg(_) => {
                    self.warn(
                        WarningKind::ResourceFallback {
                            kind: ResourceKind::Image,
                            url: Some(crate::fetch::redacted_url(url)),
                        },
                        "SVG image: an SVG document is not drawn by an SVG; the image was skipped",
                    );
                    None
                }
            },
            None => self.resources.get_decoded(url),
        }
    }

    /// The picture inside a `data:` URL, after the checks that the page makes
    /// of every inline image and of its decoded size.
    fn data_with(
        &self,
        mime: &str,
        bytes: &[u8],
        decode: impl FnOnce(&Url, Instant, u64) -> Result<Arc<DecodedImage>, Box<RenderWarning>>,
    ) -> Option<Arc<DecodedImage>> {
        {
            let mut state = self.state();
            if let Some(known) = state.cached_data(mime, bytes) {
                let found = known.lookup();
                drop(state);
                return found.resolve();
            }
            if state.attempts >= MAX_BACKGROUND_IMAGE_ATTEMPTS {
                if state.attempts == MAX_BACKGROUND_IMAGE_ATTEMPTS {
                    state.attempts += 1;
                    self.warn(
                        WarningKind::ResourceLimitExceeded {
                            kind: ResourceKind::Image,
                            limit: MAX_BACKGROUND_IMAGE_ATTEMPTS as u64,
                            actual: MAX_BACKGROUND_IMAGE_ATTEMPTS as u64 + 1,
                        },
                        "SVG image request limit was reached",
                    );
                }
                return None;
            }
            state.attempts += 1;
        }
        if self.signal.as_ref().is_some_and(AbortSignal::is_aborted) {
            return None;
        }
        let resources = self.resources;
        let warnings = SharedRenderWarnings::default();
        let stub = Url::parse("data:,").expect("an empty data URL is valid");
        let actual = bytes.len() as u64;
        let checked = resources
            .check_inline_image_url(&stub, &warnings)
            .and_then(|()| resources.check_inline_image_size(actual, &warnings))
            .and_then(|()| resources.check_inline_image_mime(&stub, mime, &warnings));
        if checked.is_none() {
            self.keep(&warnings);
            return None;
        }
        let (index, flight) = {
            let mut state = self.state();
            // Policy callbacks ran without State held. Another request may
            // have registered or completed this key while they ran.
            if let Some(known) = state.cached_data(mime, bytes) {
                let found = known.lookup();
                drop(state);
                return found.resolve();
            }
            let key_bytes = DATA_ENTRY_METADATA_BYTES
                .saturating_add(mime.len() as u64)
                .saturating_add(bytes.len() as u64);
            let lease = match InlineImageLease::new(&resources.image_cache, key_bytes) {
                Ok(lease) => lease,
                Err((limit, actual)) => {
                    drop(state);
                    self.memo_limit_warning(limit, actual);
                    return None;
                }
            };
            // This counter update calls no policy/provider code. Holding State
            // through the charge and append gives one owner for the key.
            if resources
                .charge_inline_image_budget(actual, &warnings)
                .is_none()
            {
                drop(state);
                drop(lease);
                self.keep(&warnings);
                return None;
            }
            let flight = Arc::new(DataFlight::new());
            let index = state.remember_data(mime, bytes, None, lease);
            state.data[index].pending = Some(Arc::clone(&flight));
            (index, flight)
        };
        // The owner guard outlives every State/cache guard, including on unwind.
        let pending = PendingData::new(self, index, flight);
        let start = Instant::now();
        let permit = resources.acquire_decode_permit();
        let output_limit = resources
            .image_policy_limit()
            .unwrap_or(u64::MAX)
            .min(resources.inline_image_remaining());
        let image = match decode(&stub, start, output_limit) {
            Ok(image) => Some(image),
            Err(warning) => {
                push_resource_warning(&warnings, *warning);
                None
            }
        };
        self.keep(&warnings);
        pending.finish(image, permit)
    }

    fn memo_limit_warning(&self, limit: u64, actual: u64) {
        self.warn(
            WarningKind::ResourceLimitExceeded {
                kind: ResourceKind::Image,
                limit,
                actual,
            },
            "SVG data image memo exceeded its cache byte limit",
        );
    }

    /// Reserves only fresh decoder output. Memo hits and caller clones reuse
    /// the original reservation, without charging the same pixels again.
    fn reserve_data_pixels(
        &self,
        image: Option<Arc<DecodedImage>>,
    ) -> (Option<Arc<DecodedImage>>, Option<InlineImageLease>) {
        let Some(image) = image else {
            return (None, None);
        };
        match InlineImageLease::new(&self.resources.image_cache, image.rgba.len() as u64) {
            Ok(lease) => (Some(image), Some(lease)),
            Err((limit, actual)) => {
                drop(image);
                self.memo_limit_warning(limit, actual);
                (None, None)
            }
        }
    }

    /// Publishes already-reserved pixels without holding a decode permit.
    /// Failed decodes keep the checked key, so they are not charged again.
    fn complete_data(
        &self,
        index: usize,
        image: Option<Arc<DecodedImage>>,
        pixels: Option<InlineImageLease>,
    ) -> Option<Arc<DecodedImage>> {
        let mut state = self.state();
        let entry = &mut state.data[index];
        entry.image = image.clone();
        entry._pixels_lease = pixels;
        entry.pending = None;
        image
    }
}

impl SvgImageSource for RenderSvgImages<'_, '_> {
    fn url(&self, href: &str) -> Option<Arc<DecodedImage>> {
        let Some(url) = resolve_url(href, self.base_url.as_ref()) else {
            self.warn(
                WarningKind::ResourceFallback {
                    kind: ResourceKind::Image,
                    url: None,
                },
                "SVG image: the URL could not be resolved against the document's base; the image was skipped",
            );
            return None;
        };
        let request = SharedRenderWarnings::default();
        {
            let mut state = self.state();
            let State { seen, attempts, .. } = &mut *state;
            self.resources.preload_background_urls(
                [href],
                self.base_url.as_ref(),
                &request,
                seen,
                attempts,
                self.signal.as_ref(),
            );
        }
        self.keep(&request);
        self.cached(&url)
    }

    fn data(&self, mime: &str, bytes: &[u8]) -> Option<Arc<DecodedImage>> {
        self.data_with(mime, bytes, |url, start, limit| {
            self.resources.decode_raster_image(url, bytes, start, limit)
        })
    }
}

#[cfg(test)]
mod tests;
