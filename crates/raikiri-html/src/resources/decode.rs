//! Raster decoding and sizing of CSS background images.

use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use raikiri_svg::SvgDocument;
use raikiri_traits::image::png_with_repaired_crcs;
use raikiri_traits::{
    DecodedImage, ImageIntrinsicSize, ImageRasterSize, RenderWarning, ResourceKind, WarningKind,
};
use url::Url;

use crate::fetch::redacted_url;

use super::{SharedRenderWarnings, push_resource_warning};

#[derive(Debug)]
pub(super) enum BackgroundImageDecodeError {
    Decode,
    DecodedTooLarge { limit: u64, actual: u64 },
}

pub(super) fn decode_background_raster(
    bytes: &[u8],
    max_decoded_bytes: u64,
) -> Result<DecodedImage, BackgroundImageDecodeError> {
    const DECODER_ALLOCATION_HEADROOM: u64 = 16 * 1024 * 1024;
    const DECODER_ALLOCATION_HARD_LIMIT: u64 = 512 * 1024 * 1024;

    let mut limits = Limits::default();
    limits.max_alloc = Some(
        max_decoded_bytes
            .saturating_mul(2)
            .saturating_add(DECODER_ALLOCATION_HEADROOM)
            .min(DECODER_ALLOCATION_HARD_LIMIT),
    );
    let bytes = png_with_repaired_crcs(bytes);
    let mut reader = ImageReader::new(Cursor::new(&*bytes));
    reader.limits(limits);
    let decoder = reader
        .with_guessed_format()
        .map_err(|_| BackgroundImageDecodeError::Decode)?
        .into_decoder()
        .map_err(|_| BackgroundImageDecodeError::Decode)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(BackgroundImageDecodeError::Decode);
    }
    let expected_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(BackgroundImageDecodeError::Decode)?;
    if expected_bytes > max_decoded_bytes {
        return Err(BackgroundImageDecodeError::DecodedTooLarge {
            limit: max_decoded_bytes,
            actual: expected_bytes,
        });
    }

    let rgba = DynamicImage::from_decoder(decoder)
        .map_err(|_| BackgroundImageDecodeError::Decode)?
        .into_rgba8();
    let pixels = rgba.into_raw();
    let actual_bytes = u64::try_from(pixels.len()).unwrap_or(u64::MAX);
    if actual_bytes != expected_bytes {
        return Err(BackgroundImageDecodeError::Decode);
    }
    if actual_bytes > max_decoded_bytes {
        return Err(BackgroundImageDecodeError::DecodedTooLarge {
            limit: max_decoded_bytes,
            actual: actual_bytes,
        });
    }
    Ok(DecodedImage {
        width,
        height,
        rgba: pixels,
    })
}

pub(super) fn image_intrinsic_size(image: &DecodedImage) -> ImageIntrinsicSize {
    let width = image.width as f32;
    let height = image.height as f32;
    ImageIntrinsicSize {
        width: Some(width),
        height: Some(height),
        aspect_ratio: (height > 0.0).then_some(width / height),
    }
}

pub(super) fn svg_intrinsic_size(document: &SvgDocument) -> ImageIntrinsicSize {
    let intrinsic = document.intrinsic_size();
    ImageIntrinsicSize {
        width: intrinsic.width,
        height: intrinsic.height,
        aspect_ratio: intrinsic.aspect_ratio,
    }
}

pub(super) fn default_background_raster_size(intrinsic: ImageIntrinsicSize) -> ImageRasterSize {
    const DEFAULT_WIDTH: f32 = 300.0;
    const DEFAULT_HEIGHT: f32 = 150.0;
    match (intrinsic.width, intrinsic.height, intrinsic.aspect_ratio) {
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

pub(super) fn push_image_limit_warning(
    warnings: &SharedRenderWarnings,
    limit: u64,
    actual: u64,
    details: &str,
) {
    push_resource_warning(
        warnings,
        RenderWarning {
            kind: WarningKind::ResourceLimitExceeded {
                kind: ResourceKind::Image,
                limit,
                actual,
            },
            node_id: None,
            details: details.to_owned(),
        },
    );
}

pub(super) fn push_image_fallback_warning(
    warnings: &SharedRenderWarnings,
    url: &Url,
    details: &str,
) {
    push_resource_warning(
        warnings,
        RenderWarning {
            kind: WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                url: Some(redacted_url(url)),
            },
            node_id: None,
            details: details.to_owned(),
        },
    );
}
