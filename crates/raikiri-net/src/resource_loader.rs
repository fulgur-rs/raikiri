//! Resource loading primitives used by image consumers.
//!
//! URL-scheme handling and `data:` decoding belong in this resource layer,
//! before bytes reach a format decoder. This keeps `ImageResolver` as a thin
//! replaced-element adapter and leaves the decoder independent of URLs and
//! providers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use raikiri_svg::{SvgDocument, SvgRootStyle, SvgViewport};
use raikiri_traits::{
    Body, DecodedImage, FetchedResource, ImageIntrinsicSize, ImageRasterSize, Method,
    NetworkProvider, Request, ResolverError, policy::ResourceKind,
};
use url::Url;

use crate::image_decoder::ImageDecoder;

/// Maximum encoded image input and SVG raster output size accepted by this adapter.
pub(crate) const MAX_SVG_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 256;

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

/// Loads and caches decoded image resources.
///
/// `ResourceLoader` owns provider dispatch, `data:` URL decoding, request
/// construction, and the decoded-image cache. [`ImageDecoder`] remains a pure
/// bytes-to-pixels component.
pub(crate) struct ResourceLoader<N> {
    network: N,
    decoder: ImageDecoder,
    cache: Mutex<HashMap<Url, Arc<ImageSource>>>,
    admission: Mutex<()>,
    budget: Arc<Mutex<CacheBudget>>,
    max_entries: usize,
}

pub(crate) struct ImageSource {
    kind: ImageSourceKind,
    pub(crate) intrinsic: ImageIntrinsicSize,
    raster_cache: Mutex<Option<(ImageRasterSize, Arc<DecodedImage>)>>,
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
            intrinsic: ImageIntrinsicSize {
                width: Some(width),
                height: Some(height),
                aspect_ratio: (height > 0.0).then_some(width / height),
            },
            raster_cache: Mutex::new(None),
            budget,
        }
    }

    fn svg(document: SvgDocument, bytes: Arc<[u8]>, budget: Arc<Mutex<CacheBudget>>) -> Self {
        let intrinsic = document.intrinsic_size();
        Self {
            // Retain only charged source bytes. SVG use references can expand a
            // small input into a much larger parsed tree.
            kind: ImageSourceKind::Svg(bytes),
            intrinsic: ImageIntrinsicSize {
                width: intrinsic.width,
                height: intrinsic.height,
                aspect_ratio: intrinsic.aspect_ratio,
            },
            raster_cache: Mutex::new(None),
            budget,
        }
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
                if let Some((cached_size, cached_image)) = &*cached
                    && *cached_size == size
                {
                    return max_output_bytes
                        .is_none_or(|limit| cached_image.rgba.len() as u64 <= limit)
                        .then(|| cached_image.clone());
                }
                let old_bytes = cached
                    .as_ref()
                    .map_or(0, |(_, image)| image.rgba.len() as u64);
                // Serialize allocations across all sources sharing this cache.
                // Keep the previous raster on failure. During replacement the
                // extra buffer is bounded by the SVG per-image output limit.
                let mut budget = self.budget.lock().ok()?;
                let available = budget.remaining().checked_add(old_bytes)?;
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
                budget.used = budget.used.checked_sub(old_bytes)? + image.rgba.len() as u64;
                let image = Arc::new(image);
                *cached = Some((size, image.clone()));
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
                .as_ref()
                .map(|(_, image)| image.clone()),
        }
    }
}

impl<N: NetworkProvider> ResourceLoader<N> {
    pub(crate) fn new(network: N) -> Self {
        Self::with_limits(network, MAX_CACHE_BYTES, MAX_CACHE_ENTRIES)
    }

    fn with_limits(network: N, max_bytes: u64, max_entries: usize) -> Self {
        Self {
            network,
            decoder: ImageDecoder,
            cache: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            budget: Arc::new(Mutex::new(CacheBudget {
                used: 0,
                limit: max_bytes,
            })),
            max_entries,
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
        self.cache.lock().ok()?.get(url).cloned()
    }

    pub(crate) fn load_source(&self, url: &Url) -> Result<Arc<ImageSource>, ResolverError> {
        if let Some(cached) = self.cached_source(url) {
            return Ok(cached);
        }
        // Bound concurrent misses and coalesce duplicate fetches without
        // blocking cache-hit lookups for the duration of a network request.
        let _admission = self.admission.lock().map_err(|_| cache_limit_error())?;
        if let Some(cached) = self.cached_source(url) {
            return Ok(cached);
        }
        if self.cache.lock().map_err(|_| cache_limit_error())?.len() >= self.max_entries {
            return Err(cache_limit_error());
        }
        let key_bytes = url.as_str().len() as u64;
        if key_bytes
            > self
                .budget
                .lock()
                .map_err(|_| cache_limit_error())?
                .remaining()
        {
            return Err(cache_limit_error());
        }

        let (bytes, is_data_url, is_svg) = if url.scheme() == "data" {
            let is_svg = data_url_media_type(url.as_str()).is_some_and(is_svg_media_type);
            (decode_data_url(url.as_str())?, true, is_svg)
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
            (fetched.bytes.to_vec(), false, is_svg)
        };
        if bytes.len() > MAX_SVG_BYTES {
            return Err(ResolverError::Decode(
                "image input exceeds byte limit".to_owned(),
            ));
        }

        // Charge retained URL keys (including data URLs) and SVG source bytes
        // as well as pixels; otherwise tiny or unrasterized images evade the
        // aggregate byte limit. Entry count also bounds per-source overhead.
        let source_bytes = key_bytes
            .checked_add(if is_svg { bytes.len() as u64 } else { 0 })
            .ok_or_else(cache_limit_error)?;
        let mut budget = self.budget.lock().map_err(|_| cache_limit_error())?;
        let available = budget
            .remaining()
            .checked_sub(source_bytes)
            .ok_or_else(cache_limit_error)?;
        let source = if is_svg {
            let document = SvgDocument::parse(&bytes)
                .map_err(|error| ResolverError::Decode(format!("SVG parse failed: {error}")))?;
            Arc::new(ImageSource::svg(
                document,
                bytes.into(),
                self.budget.clone(),
            ))
        } else {
            let decoded = self.decoder.decode(&bytes, available).map_err(|error| {
                if is_data_url {
                    ResolverError::Decode(format!("data URL image decode failed: {error}"))
                } else {
                    ResolverError::Decode(error)
                }
            })?;
            Arc::new(ImageSource::raster(Arc::new(decoded), self.budget.clone()))
        };
        let mut cache = self.cache.lock().map_err(|_| cache_limit_error())?;
        budget.used += source_bytes + source.decoded_byte_len().unwrap_or(0);
        cache.insert(url.clone(), source.clone());
        Ok(source)
    }

    fn fetch(&self, url: &Url) -> Result<FetchedResource, ResolverError> {
        self.network
            .fetch(Request {
                url: url.clone(),
                method: Method::Get,
                content_type: None,
                headers: Vec::new(),
                body: Body::Empty,
                signal: None,
                kind: ResourceKind::Image,
            })
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
