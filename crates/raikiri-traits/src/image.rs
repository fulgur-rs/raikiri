//! Decoded raster image data + paint-time pixel access.
//!
//! Kept separate from [`crate::resolver::ReplacedResolver`] (which resolves
//! intrinsic *size* only) so a paint-only consumer never needs a full
//! resolver, and so `ReplacedResolver` itself never grows a paint-shaped
//! dependency. A single concrete type (e.g. an image cache) is expected to
//! implement both traits and be handed to layout and paint separately.

use std::sync::Arc;

use url::Url;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptySource;
    impl ImagePixelSource for EmptySource {
        fn get_decoded(&self, _url: &Url) -> Option<Arc<DecodedImage>> {
            None
        }
    }

    #[test]
    fn image_pixel_source_is_object_safe() {
        fn _assert<T: ?Sized>() {}
        _assert::<dyn ImagePixelSource>();
    }

    #[test]
    fn empty_source_returns_none() {
        let url = Url::parse("file:///tmp/x.png").unwrap();
        assert!(EmptySource.get_decoded(&url).is_none());
    }
}
