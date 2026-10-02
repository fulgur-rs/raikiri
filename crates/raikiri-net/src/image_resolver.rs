//! `ImageResolver` — the `<img>` adapter over resource loading and decoding.

use std::sync::Arc;

use raikiri_traits::{
    DecodedImage, ImageIntrinsicSize, ImagePixelSource, ImageRasterSize, IntrinsicBox,
    NetworkProvider, ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError,
    ResolverRequest,
};
use url::Url;

use crate::resource_loader::{ResourceLoader, default_object_size};

/// `ReplacedResolver` + `ImagePixelSource` for `<img>` elements.
///
/// Resource loading, URL-scheme handling, source caching, and raster decoding
/// are delegated to separate internal layers. JPEG, GIF, PNG, and WebP bytes
/// are normalized to the shared RGBA8 image contract; SVG sources retain
/// natural metadata and rasterize on demand at the requested paint size.
///
/// Fetch + decode happen synchronously inside [`ReplacedResolver::resolve`];
/// results are cached by URL so later metadata and paint lookups are cache hits.
/// Each resolver retains at most 256 sources and accounts for at most 128 MiB
/// of RGBA pixels, URL keys, and SVG source bytes. New resources that exceed
/// either limit return [`ResolverError::Decode`]; existing cache entries remain
/// available. Encoded image inputs are limited to 32 MiB, and SVG rasterization
/// shares the same cache byte budget. Parsed SVG trees are temporary, so SVG
/// reference expansion does not create unaccounted retained trees.
pub struct ImageResolver<N> {
    resources: ResourceLoader<N>,
}

impl<N: NetworkProvider> ImageResolver<N> {
    /// Wraps `network` with raster and SVG image resource loading.
    pub fn new(network: N) -> Self {
        Self {
            resources: ResourceLoader::new(network),
        }
    }
}

impl<N: NetworkProvider> ReplacedResolver for ImageResolver<N> {
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        let source = self.resources.load_source(req.url())?;
        let size = default_object_size(source.intrinsic);
        let mut intrinsic = IntrinsicBox::new(size.width, size.height);
        intrinsic.aspect_ratio = source.intrinsic.aspect_ratio;
        Ok(ResolvedIntrinsic {
            intrinsic,
            disposition: ResolveDisposition::Ok,
        })
    }
}

impl<N: NetworkProvider> ImagePixelSource for ImageResolver<N> {
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>> {
        let source = self.resources.cached_source(url)?;
        let size = default_object_size(source.intrinsic);
        source.rasterize(size, None)
    }

    fn intrinsic_size(&self, url: &Url) -> Option<ImageIntrinsicSize> {
        Some(self.resources.cached_source(url)?.intrinsic)
    }

    fn decoded_byte_len(&self, url: &Url) -> Option<u64> {
        self.resources.cached_source(url)?.decoded_byte_len()
    }

    fn get_decoded_at_size(
        &self,
        url: &Url,
        size: ImageRasterSize,
        max_output_bytes: Option<u64>,
    ) -> Option<Arc<DecodedImage>> {
        self.resources
            .cached_source(url)?
            .rasterize(size, max_output_bytes)
    }
}

#[cfg(test)]
mod tests;
