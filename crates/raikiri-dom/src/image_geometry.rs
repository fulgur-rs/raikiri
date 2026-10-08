//! Shared used geometry for replaced raster objects.

use raikiri_style::property::ObjectFit;
use raikiri_style::resolve::{
    ComputedCssPosition, ComputedCssPositionOffset, ComputedLengthPercentage,
};
use raikiri_traits::ImageIntrinsicSize;

/// The object rectangle inside a used content box, in CSS pixels.
///
/// Input and output tuples contain x, y, width, and height. Double precision
/// preserves the native painter's coordinates; page consumers can convert
/// them to their neutral rectangle type. Empty or non-finite boxes are omitted.
pub fn object_image_rect(
    content: (f64, f64, f64, f64),
    natural: ImageIntrinsicSize,
    object_fit: ObjectFit,
    object_position: &ComputedCssPosition,
) -> Option<(f64, f64, f64, f64)> {
    let (x, y, width, height) = content;
    if ![x, y, width, height].into_iter().all(f64::is_finite) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let (natural_w, natural_h) = natural_object_size(natural);
    let (image_w, image_h) = match object_fit {
        ObjectFit::Contain => {
            let scale = (width / natural_w).min(height / natural_h);
            (natural_w * scale, natural_h * scale)
        }
        ObjectFit::Cover => {
            let scale = (width / natural_w).max(height / natural_h);
            (natural_w * scale, natural_h * scale)
        }
        ObjectFit::None => (natural_w, natural_h),
        ObjectFit::ScaleDown => {
            let scale = (width / natural_w).min(height / natural_h).min(1.0);
            (natural_w * scale, natural_h * scale)
        }
        _ => (width, height),
    };
    let image_x = x + position_offset(object_position.horizontal, width - image_w);
    let image_y = y + position_offset(object_position.vertical, height - image_h);
    ([image_x, image_y, image_w, image_h]
        .into_iter()
        .all(f64::is_finite)
        && image_w > 0.0
        && image_h > 0.0)
        .then_some((image_x, image_y, image_w, image_h))
}

fn natural_object_size(natural: ImageIntrinsicSize) -> (f64, f64) {
    const DEFAULT_WIDTH: f64 = 300.0;
    const DEFAULT_HEIGHT: f64 = 150.0;
    let valid = |value: Option<f32>| {
        value
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(f64::from)
    };
    let width = valid(natural.width);
    let height = valid(natural.height);
    let ratio = natural
        .aspect_ratio
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
        .map(f64::from)
        .or_else(|| width.zip(height).map(|(w, h)| w / h));
    match (width, height, ratio) {
        (Some(width), Some(height), _) => (width, height),
        (Some(width), None, Some(ratio)) => (width, width / ratio),
        (None, Some(height), Some(ratio)) => (height * ratio, height),
        (None, None, Some(ratio)) => {
            let width = DEFAULT_WIDTH.min(DEFAULT_HEIGHT * ratio);
            (width, width / ratio)
        }
        (Some(width), None, _) => (width, DEFAULT_HEIGHT),
        (None, Some(height), _) => (DEFAULT_WIDTH, height),
        _ => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    }
}

/// Resolve one position edge against the free space on its axis.
pub fn position_offset(offset: ComputedCssPositionOffset, free_space: f64) -> f64 {
    match offset {
        ComputedCssPositionOffset::Start(value) => match value {
            ComputedLengthPercentage::Px(px) => px as f64,
            ComputedLengthPercentage::Percent(percent) => free_space * percent as f64 / 100.0,
        },
        ComputedCssPositionOffset::End(value) => match value {
            ComputedLengthPercentage::Px(px) => free_space - px as f64,
            ComputedLengthPercentage::Percent(percent) => {
                free_space * (1.0 - percent as f64 / 100.0)
            }
        },
        _ => free_space / 2.0, // cov:ignore: defensive fallback for future position variants
    }
}

#[cfg(test)]
mod tests;
