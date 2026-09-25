//! Decoded raster image data + paint-time pixel access.
//!
//! Kept separate from [`crate::resolver::ReplacedResolver`] (which resolves
//! intrinsic *size* only) so a paint-only consumer never needs a full
//! resolver, and so `ReplacedResolver` itself never grows a paint-shaped
//! dependency. A single concrete type (e.g. an image cache) is expected to
//! implement both traits and be handed to layout and paint separately.

use std::sync::Arc;

use url::Url;

/// Natural image dimensions, independent of the concrete box used by layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageIntrinsicSize {
    /// Natural width in CSS pixels, when supplied by the source.
    pub width: Option<f32>,
    /// Natural height in CSS pixels, when supplied by the source.
    pub height: Option<f32>,
    /// Natural width divided by natural height, when known.
    pub aspect_ratio: Option<f32>,
}

/// Raster viewport requested by the paint stage, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageRasterSize {
    /// Raster width in CSS pixels.
    pub width: f32,
    /// Raster height in CSS pixels.
    pub height: f32,
}

/// Decoded raster image: straight (non-premultiplied) RGBA8 pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedImage {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// `width * height * 4` bytes, row-major, RGBA8, straight alpha.
    pub rgba: Vec<u8>,
}

/// Paint-time access to images already resolved via
/// [`crate::resolver::ReplacedResolver`].
pub trait ImagePixelSource {
    /// Returns the decoded pixels for `url`, if already resolved.
    ///
    /// `None` means either the URL was never resolved (e.g. `resolve()` was
    /// never called for it) or resolution failed — paint has no fallback
    /// path in this scope and simply skips drawing that element's image.
    fn get_decoded(&self, url: &Url) -> Option<Arc<DecodedImage>>;

    /// Returns natural source dimensions, if available.
    ///
    /// The default derives them from decoded pixels, which preserves the
    /// behavior of existing raster image sources. Vector sources can override
    /// this to retain missing dimensions and a known aspect ratio.
    fn intrinsic_size(&self, url: &Url) -> Option<ImageIntrinsicSize> {
        let image = self.get_decoded(url)?;
        let width = image.width as f32;
        let height = image.height as f32;
        Some(ImageIntrinsicSize {
            width: Some(width),
            height: Some(height),
            aspect_ratio: (height > 0.0).then_some(width / height),
        })
    }

    /// Reports a cached raster's decoded byte length without requesting a
    /// size-specific raster. Sources whose output size is deferred may return
    /// `None` until paint selects a concrete viewport.
    fn decoded_byte_len(&self, url: &Url) -> Option<u64> {
        self.get_decoded(url).map(|image| image.rgba.len() as u64)
    }

    /// Returns decoded pixels appropriate for a concrete paint size.
    ///
    /// Existing sources default to their original decoded pixels. The byte
    /// limit is checked before returning them; vector sources can override
    /// this method to rasterize at `size`.
    fn get_decoded_at_size(
        &self,
        url: &Url,
        _size: ImageRasterSize,
        max_output_bytes: Option<u64>,
    ) -> Option<Arc<DecodedImage>> {
        let image = self.get_decoded(url)?;
        if max_output_bytes.is_some_and(|limit| image.rgba.len() as u64 > limit) {
            return None;
        }
        Some(image)
    }
}

#[cfg(test)]
mod tests;
