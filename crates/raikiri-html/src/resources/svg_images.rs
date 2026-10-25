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
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use raikiri_svg::SvgImageSource;
use raikiri_traits::{
    AbortSignal, DecodedImage, ImagePixelSource, RenderWarning, ResourceKind, WarningKind,
};
use url::Url;

use super::{
    CachedBackgroundSource, MAX_BACKGROUND_IMAGE_ATTEMPTS, RenderResources, SharedRenderWarnings,
    push_resource_warning,
};
use crate::fetch::{resolve_url, url_without_fragment};

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
pub struct RenderSvgImages<'r, 'a> {
    resources: &'r RenderResources<'a>,
    base_url: Option<Url>,
    signal: Option<AbortSignal>,
    warnings: SharedRenderWarnings,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The URLs already requested, without fragment.
    seen: HashSet<Url>,
    /// The requests made, `data:` pictures included.
    attempts: usize,
    /// The `data:` pictures that passed the byte checks, whether they decoded
    /// or not: asking again must not charge the page's budget again.
    data: Vec<DataPicture>,
}

struct DataPicture {
    mime: String,
    bytes: Vec<u8>,
    image: Option<Arc<DecodedImage>>,
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
    fn decode_data(&self, mime: &str, bytes: &[u8]) -> Option<Arc<DecodedImage>> {
        let resources = self.resources;
        let warnings = SharedRenderWarnings::default();
        let stub = Url::parse("data:,").expect("an empty data URL is valid");
        let actual = bytes.len() as u64;
        let checked = resources
            .check_inline_image_url(&stub, &warnings)
            .and_then(|()| resources.check_inline_image_size(actual, &warnings))
            .and_then(|()| resources.check_inline_image_mime(&stub, mime, &warnings))
            .and_then(|()| resources.charge_inline_image_budget(actual, &warnings));
        if checked.is_none() {
            self.keep(&warnings);
            return None;
        }
        let start = Instant::now();
        let _permit = resources.acquire_decode_permit();
        let output_limit = resources
            .image_policy_limit()
            .unwrap_or(u64::MAX)
            .min(resources.image_cache_effective_remaining(&stub));
        let image = match resources.decode_raster_image(&stub, bytes, start, output_limit) {
            Ok(image) => Some(image),
            Err(warning) => {
                push_resource_warning(&warnings, *warning);
                None
            }
        };
        self.keep(&warnings);
        // The budget has been charged: remember the outcome, so that a second
        // scene with the same picture neither charges nor decodes again.
        self.state().data.push(DataPicture {
            mime: mime.to_owned(),
            bytes: bytes.to_vec(),
            image: image.clone(),
        });
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
        {
            let mut state = self.state();
            if let Some(known) = state
                .data
                .iter()
                .find(|known| known.mime == mime && known.bytes == bytes)
            {
                return known.image.clone();
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
        self.decode_data(mime, bytes)
    }
}
