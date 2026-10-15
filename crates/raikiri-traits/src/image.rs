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

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Returns `bytes` with every PNG chunk CRC rewritten to match its chunk.
///
/// Browsers draw PNGs whose chunk CRCs are wrong (Chromium sets libpng's
/// `PNG_CRC_QUIET_USE` for both critical and ancillary chunks), while the
/// `png` crate rejects them. Rewriting the CRCs before decoding gives the
/// browser behavior without changing anything else: the zlib Adler-32 check
/// and every structural check still apply. Bytes that are not a PNG, or that
/// need no change, are returned borrowed. A truncated trailing chunk is left
/// as is for the decoder to report.
#[must_use]
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
