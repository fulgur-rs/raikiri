//! Decoded raster image data + paint-time pixel access.
//!
//! Kept separate from [`crate::resolver::ReplacedResolver`] (which resolves
//! intrinsic *size* only) so a paint-only consumer never needs a full
//! resolver, and so `ReplacedResolver` itself never grows a paint-shaped
//! dependency. A single concrete type (e.g. an image cache) is expected to
//! implement both traits and be handed to layout and paint separately.

use std::borrow::Cow;
use std::sync::Arc;

use url::Url;

use crate::vector::VectorImage;

/// Natural image dimensions, independent of the concrete box used by layout.
///
/// The default knows nothing: no width, no height and no ratio.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[non_exhaustive]
pub struct ImageIntrinsicSize {
    /// Natural width in CSS pixels, when supplied by the source.
    pub width: Option<f32>,
    /// Natural height in CSS pixels, when supplied by the source.
    pub height: Option<f32>,
    /// Natural width divided by natural height, when known.
    pub aspect_ratio: Option<f32>,
}

impl ImageIntrinsicSize {
    /// The dimensions a source supplies; any of them may be missing.
    pub fn new(width: Option<f32>, height: Option<f32>, aspect_ratio: Option<f32>) -> Self {
        Self {
            width,
            height,
            aspect_ratio,
        }
    }
}

/// Raster viewport requested by the paint stage, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageRasterSize {
    /// Raster width in CSS pixels.
    pub width: f32,
    /// Raster height in CSS pixels.
    pub height: f32,
}

/// What the paint stage asks of [`ImagePixelSource::get_vector`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ImageVectorRequest {
    /// The box the image is drawn in, in CSS pixels. Percentage lengths and
    /// `preserveAspectRatio` of the image resolve against it.
    pub size: ImageRasterSize,
}

impl ImageVectorRequest {
    /// A request for the image drawn in a box of `size`.
    pub fn new(size: ImageRasterSize) -> Self {
        Self { size }
    }
}

/// Decoded raster image: straight (non-premultiplied) RGBA8 pixels.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DecodedImage {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// `width * height * 4` bytes, row-major, RGBA8, straight alpha.
    pub rgba: Vec<u8>,
}

impl DecodedImage {
    /// An image of `width` x `height` pixels held in `rgba`, which must hold
    /// `width * height * 4` bytes.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self {
            width,
            height,
            rgba,
        }
    }
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

    /// Returns the image as a vector drawing for the box of `request`.
    ///
    /// The default is `None`: the source has no vector form of `url`, or does
    /// not support one. A caller then asks for pixels with
    /// [`Self::get_decoded_at_size`], which keeps every existing source
    /// working. A source also answers `None` for a picture it can draw only
    /// as pixels, so the answer is a choice of the source, never a failure of
    /// the image.
    fn get_vector(&self, _url: &Url, _request: &ImageVectorRequest) -> Option<Arc<VectorImage>> {
        None
    }
}

/// Converts straight-alpha RGBA8 `pixels` from the color space of an embedded
/// ICC `profile` to sRGB, in place, and reports whether any pixel was rewritten.
///
/// CSS Color 4 treats a tagged image as being in the color space its valid
/// profile describes, and an untagged image, or one whose profile cannot be
/// used, as sRGB (<https://drafts.csswg.org/css-color-4/#tagged-images>).
/// The pixels stay as they are when the profile cannot be parsed, is not an
/// RGB profile, or already lands within one 8-bit step of sRGB on a probe of
/// gray and single-channel ramps and a grid of mixed colors. That last case
/// keeps the many images tagged with an sRGB profile byte for byte instead of
/// adding rounding noise to them. Alpha is never touched.
#[doc(hidden)]
pub fn convert_rgba8_to_srgb(profile: &[u8], pixels: &mut [u8]) -> bool {
    use moxcms::{ColorProfile, DataColorSpace, Layout, RenderingIntent, TransformOptions};

    // Pixels per pass; bounds the scratch copy the transform reads from.
    const CHUNK_BYTES: usize = 4 * 16 * 1024;

    if pixels.is_empty() || !pixels.len().is_multiple_of(4) {
        return false;
    }
    let Some(source) = ColorProfile::new_from_slice(profile)
        .ok()
        .filter(|source| source.color_space == DataColorSpace::Rgb)
    else {
        return false;
    };
    let srgb = ColorProfile::new_srgb();
    // A profile built from lookup tables may carry them for some rendering
    // intents only, and many carry just the perceptual one. Prefer the
    // relative colorimetric table, and settle for whichever one is there.
    let transform = [
        RenderingIntent::RelativeColorimetric,
        RenderingIntent::Perceptual,
        RenderingIntent::Saturation,
    ]
    .into_iter()
    .find_map(|rendering_intent| {
        let options = TransformOptions {
            rendering_intent,
            ..TransformOptions::default()
        };
        source
            .create_transform_8bit(Layout::Rgba, &srgb, Layout::Rgba, options)
            .ok()
    });
    let Some(transform) = transform else {
        return false;
    };

    let probe = srgb_probe_pixels();
    let mut probed = vec![0u8; probe.len()];
    if transform.transform(&probe, &mut probed).is_ok()
        && probe
            .iter()
            .zip(&probed)
            .all(|(before, after)| before.abs_diff(*after) <= 1)
    {
        return false;
    }

    let mut scratch = [0u8; CHUNK_BYTES];
    pixels.chunks_mut(CHUNK_BYTES).all(|chunk| {
        let source = &mut scratch[..chunk.len()];
        source.copy_from_slice(chunk);
        transform.transform(source, chunk).is_ok()
    })
}

/// Opaque RGBA8 colors that tell an sRGB profile from any other: grays and
/// single-channel ramps in steps of five, then a grid of 17 levels per
/// channel across mixed colors. The ramps alone are not enough, because a
/// profile built from a color lookup table can leave them as they are and
/// still move the colors between them.
fn srgb_probe_pixels() -> Vec<u8> {
    let ramps = (0..=255u8).step_by(5).flat_map(|level| {
        [
            [level, level, level, 255],
            [level, 0, 0, 255],
            [0, level, 0, 255],
            [0, 0, level, 255],
        ]
    });
    let levels = || (0..=240u8).step_by(16).chain([255]);
    let grid = levels().flat_map(|red| {
        levels().flat_map(move |green| levels().map(move |blue| [red, green, blue, 255]))
    });
    ramps.chain(grid).flatten().collect()
}

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Returns `bytes` with every PNG chunk CRC rewritten to match its chunk.
///
/// Browsers draw PNGs whose chunk CRCs are wrong (Chromium sets libpng's
/// `PNG_CRC_QUIET_USE` for both critical and ancillary chunks), while the
/// `png` crate rejects them. Rewriting the CRCs before decoding gives the
/// browser behavior without changing anything else: every structural and
/// zlib stream check still applies. The zlib Adler-32 trailer is not a
/// safeguard here, because the `png` crate skips it by default. Bytes that
/// are not a PNG, or that need no change, are returned borrowed. A truncated
/// trailing chunk is left as is for the decoder to report.
#[must_use]
#[doc(hidden)]
pub fn png_with_repaired_crcs(bytes: &[u8]) -> Cow<'_, [u8]> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Cow::Borrowed(bytes);
    }
    let mut repaired: Option<Vec<u8>> = None;
    let mut at = PNG_SIGNATURE.len();
    while let Some(header) = bytes.get(at..at + 8) {
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let crc_at = (at + 8).saturating_add(length);
        let Some(stored) = crc_at.checked_add(4).and_then(|end| bytes.get(crc_at..end)) else {
            break;
        };
        let expected = crc32(&bytes[at + 4..crc_at]).to_be_bytes();
        if stored != expected {
            repaired.get_or_insert_with(|| bytes.to_vec())[crc_at..crc_at + 4]
                .copy_from_slice(&expected);
        }
        if &header[4..8] == b"IEND" {
            break;
        }
        at = crc_at + 4;
    }
    repaired.map_or(Cow::Borrowed(bytes), Cow::Owned)
}

/// CRC-32 as defined for PNG chunks (ISO 3309, reflected polynomial
/// `0xEDB88320`), computed over the chunk type and data.
fn crc32(bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut c = n as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 == 1 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            table[n] = c;
            n += 1;
        }
        table
    };
    !bytes.iter().fold(u32::MAX, |crc, &byte| {
        TABLE[((crc ^ u32::from(byte)) & 0xFF) as usize] ^ (crc >> 8)
    })
}

#[cfg(test)]
mod tests;
