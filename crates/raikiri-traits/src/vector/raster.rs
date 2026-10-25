//! Raster pictures in a vector scene.

use std::sync::Arc;

use crate::image::DecodedImage;

/// A decoded picture drawn inside a vector scene.
///
/// The picture covers the rectangle from `(0, 0)` to `(width, height)` of the
/// space of its enclosing group, one pixel to one unit, so the transform of
/// that group sizes and places it. The group carries the opacity, the clip of
/// a `preserveAspectRatio` slice and everything else that applies to the
/// picture as a whole.
///
/// The pixels are shared with whoever decoded them, usually the page's image
/// cache, so a scene that shows one picture many times holds it once.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RasterImage {
    /// The pixels, straight RGBA8.
    pub image: Arc<DecodedImage>,
    /// Whether scaling interpolates between pixels. When it is false each
    /// destination pixel takes the nearest source pixel, as `image-rendering:
    /// pixelated` and `crisp-edges` ask for.
    pub smooth: bool,
}

impl RasterImage {
    /// A picture that scaling smooths.
    pub fn new(image: Arc<DecodedImage>) -> Self {
        Self {
            image,
            smooth: true,
        }
    }
}
