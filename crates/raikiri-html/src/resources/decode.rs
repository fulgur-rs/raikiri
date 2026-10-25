//! Raster decoding and sizing of CSS background images.

use std::io::Cursor;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use raikiri_svg::SvgDocument;
use raikiri_traits::image::{convert_rgba8_to_srgb, png_with_repaired_crcs};
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
    let mut decoder = reader
        .with_guessed_format()
        .map_err(|_| BackgroundImageDecodeError::Decode)?
        .into_decoder()
        .map_err(|_| BackgroundImageDecodeError::Decode)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(BackgroundImageDecodeError::Decode);
    }
    // `image-orientation` is `from-image` unless the author says otherwise
    // (<https://drafts.csswg.org/css-images-3/#propdef-image-orientation>),
    // so a tag in the file's EXIF data turns the image before anything sizes
    // or paints it. A tag the decoder cannot read leaves the image as stored.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    // A tagged image is in the color space of its embedded profile, which is
    // converted to sRGB once the pixels are decoded.
    let icc_profile = decoder.icc_profile().ok().flatten();
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

    let mut image =
        DynamicImage::from_decoder(decoder).map_err(|_| BackgroundImageDecodeError::Decode)?;
    image.apply_orientation(orientation);
    // A quarter turn swaps the sides.
    let (width, height) = (image.width(), image.height());
    let mut pixels = image.into_rgba8().into_raw();
    if let Some(profile) = icc_profile {
        convert_rgba8_to_srgb(&profile, &mut pixels);
    }
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
    Ok(DecodedImage::new(width, height, pixels))
}

pub(super) fn image_intrinsic_size(image: &DecodedImage) -> ImageIntrinsicSize {
    let width = image.width as f32;
    let height = image.height as f32;
    ImageIntrinsicSize::new(
        Some(width),
        Some(height),
        (height > 0.0).then_some(width / height),
    )
}

pub(super) fn svg_intrinsic_size(document: &SvgDocument) -> ImageIntrinsicSize {
    let intrinsic = document.intrinsic_size();
    ImageIntrinsicSize::new(intrinsic.width, intrinsic.height, intrinsic.aspect_ratio)
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
        RenderWarning::new(
            WarningKind::ResourceLimitExceeded {
                kind: ResourceKind::Image,
                limit,
                actual,
            },
            details.to_owned(),
        ),
    );
}

pub(super) fn push_image_fallback_warning(
    warnings: &SharedRenderWarnings,
    url: &Url,
    details: &str,
) {
    push_resource_warning(
        warnings,
        RenderWarning::new(
            WarningKind::ResourceFallback {
                kind: ResourceKind::Image,
                url: Some(redacted_url(url)),
            },
            details.to_owned(),
        ),
    );
}
