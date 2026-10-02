//! Pure image-byte decoding.

use std::io::Cursor;

use image::{DynamicImage, ImageDecoder as _, ImageReader, Limits};
use raikiri_traits::DecodedImage;

/// Decodes supported raster bytes into the shared straight RGBA8 contract.
///
/// The decoder receives bytes only. URL schemes, MIME headers, fetching, and
/// resource policy are intentionally outside this type.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ImageDecoder;

impl ImageDecoder {
    pub(crate) fn decode(
        &self,
        bytes: &[u8],
        max_output_bytes: u64,
    ) -> Result<DecodedImage, String> {
        let mut limits = Limits::default();
        limits.max_alloc = Some(
            max_output_bytes
                .saturating_mul(2)
                .saturating_add(16 * 1024 * 1024)
                .min(512 * 1024 * 1024),
        );
        let mut reader = ImageReader::new(Cursor::new(bytes));
        reader.limits(limits);
        let decoder = reader
            .with_guessed_format()
            .map_err(|error| error.to_string())?
            .into_decoder()
            .map_err(|error| error.to_string())?;
        let (width, height) = decoder.dimensions();
        let output_bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "decoded image dimensions overflow byte count".to_owned())?;
        if width == 0 || height == 0 || output_bytes > max_output_bytes {
            return Err("decoded image exceeds cache byte budget".to_owned());
        }
        let rgba = DynamicImage::from_decoder(decoder)
            .map_err(|error| error.to_string())?
            .into_rgba8();
        let width = rgba.width();
        let height = rgba.height();
        let pixels = rgba.into_raw();
        // Defensive invariant check: `DecodedImage::rgba`'s contract (see
        // raikiri-traits::image) is exactly `width * height * 4` bytes. Catches
        // any future decoder/transformation combination that slips past the
        // normalization step without producing the shape expected by paint.
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "decoded image dimensions overflow address space".to_string())?;
        // cov:ignore: image crate guarantees the normalized buffer length
        if pixels.len() != expected_len {
            return Err(format!(
                "decoded image pixel buffer size mismatch: expected {expected_len} bytes, got {}",
                pixels.len()
            ));
        }
        Ok(DecodedImage {
            width,
            height,
            rgba: pixels,
        })
    }
}

#[cfg(test)]
mod tests;
