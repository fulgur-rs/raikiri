//! Shared used geometry for replaced raster objects and background images.

use raikiri_style::ComputedBackgroundSize;
use raikiri_style::property::{BackgroundRepeat, BackgroundRepeatKeyword, ObjectFit};
use raikiri_style::resolve::{
    ComputedCssPosition, ComputedCssPositionOffset, ComputedLengthPercentage,
    ComputedLengthPercentageOrAuto,
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

fn background_length(value: ComputedLengthPercentageOrAuto, basis: f64) -> Option<f64> {
    match value {
        ComputedLengthPercentageOrAuto::Px(px) => Some(px as f64),
        ComputedLengthPercentageOrAuto::Percent(percent) => Some(basis * percent as f64 / 100.0),
        ComputedLengthPercentageOrAuto::Auto | ComputedLengthPercentageOrAuto::MinContent => None,
        ComputedLengthPercentageOrAuto::Calc(calc) => {
            Some(basis * calc.percent as f64 / 100.0 + calc.px as f64)
        }
    }
}

/// The used size of a CSS background image (CSS Backgrounds 3 §3.9), in CSS
/// pixels.
///
/// `area_w` and `area_h` are the background positioning area, which
/// `background-size` percentages, `cover`, and `contain` resolve against.
/// Missing natural dimensions fall back to the known ratio or the default
/// object size of 300×150. Empty or non-finite results are omitted.
pub fn background_image_dimensions(
    size: &ComputedBackgroundSize,
    area_w: f64,
    area_h: f64,
    intrinsic: ImageIntrinsicSize,
) -> Option<(f64, f64)> {
    const DEFAULT_WIDTH: f64 = 300.0;
    const DEFAULT_HEIGHT: f64 = 150.0;

    let (intrinsic_w, intrinsic_h) = match (
        intrinsic.width.map(f64::from),
        intrinsic.height.map(f64::from),
        intrinsic.aspect_ratio.map(f64::from),
    ) {
        (Some(width), Some(height), _) if width > 0.0 && height > 0.0 => (width, height),
        (Some(width), None, Some(ratio)) if width > 0.0 && ratio > 0.0 => (width, width / ratio),
        (None, Some(height), Some(ratio)) if height > 0.0 && ratio > 0.0 => {
            (height * ratio, height)
        }
        (None, None, Some(ratio)) if ratio > 0.0 && area_w > 0.0 && area_h > 0.0 => {
            let width = area_w.min(area_h * ratio);
            (width, width / ratio)
        }
        (None, None, Some(ratio)) if ratio > 0.0 => {
            let width = DEFAULT_WIDTH.min(DEFAULT_HEIGHT * ratio);
            (width, width / ratio)
        }
        (Some(width), None, _) if width > 0.0 => (width, DEFAULT_HEIGHT),
        (None, Some(height), _) if height > 0.0 => (DEFAULT_WIDTH, height),
        _ => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    };
    let (image_w, image_h) = match size {
        ComputedBackgroundSize::Cover => {
            let scale = (area_w / intrinsic_w).max(area_h / intrinsic_h);
            (intrinsic_w * scale, intrinsic_h * scale)
        }
        ComputedBackgroundSize::Contain => {
            let scale = (area_w / intrinsic_w).min(area_h / intrinsic_h);
            (intrinsic_w * scale, intrinsic_h * scale)
        }
        ComputedBackgroundSize::Explicit { width, height } => {
            let width = background_length(*width, area_w);
            let height = background_length(*height, area_h);
            match (width, height) {
                (Some(width), Some(height)) => (width.max(0.0), height.max(0.0)),
                (Some(width), None) => (width.max(0.0), width.max(0.0) * intrinsic_h / intrinsic_w),
                (None, Some(height)) => {
                    (height.max(0.0) * intrinsic_w / intrinsic_h, height.max(0.0))
                }
                (None, None) => (intrinsic_w, intrinsic_h),
            }
        }
        _ => (intrinsic_w, intrinsic_h), // cov:ignore: defensive fallback for future background-size variants
    };
    (image_w.is_finite() && image_h.is_finite() && image_w > 0.0 && image_h > 0.0)
        .then_some((image_w, image_h))
}

/// The tiles of one CSS background image layer, in CSS pixels.
///
/// Every tile has the same size; a tile's origin is one entry of `x` paired
/// with one entry of `y`. The tiles cover the painting area and are clipped to
/// it by the painter.
#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundTiles {
    /// Tile width after `round` rescaling.
    pub width: f64,
    /// Tile height after `round` rescaling.
    pub height: f64,
    /// Left edges of the tile columns.
    pub x: Vec<f64>,
    /// Top edges of the tile rows.
    pub y: Vec<f64>,
}

/// Place the tiles of one background image (CSS Backgrounds 3 §3.4–§3.6).
///
/// `positioning` is the `background-origin` box and `painting` the
/// `background-clip` box, both as x, y, width, and height. `image` is the
/// used size from [`background_image_dimensions`]. `repeat` tiles the origin
/// tile in both directions to cover the painting area; `no-repeat` places
/// only the origin tile; `space` pins the first and last tiles to the
/// positioning edges with even gaps; `round` rescales the tile so a whole
/// number fills the positioning area. Axes combine independently. Returns
/// `None` when nothing can be painted.
pub fn background_tiles(
    positioning: (f64, f64, f64, f64),
    painting: (f64, f64, f64, f64),
    image: (f64, f64),
    position: &ComputedCssPosition,
    repeat: &BackgroundRepeat,
) -> Option<BackgroundTiles> {
    let (pos_x, pos_y, pos_w, pos_h) = positioning;
    let (paint_x, paint_y, paint_w, paint_h) = painting;
    let (image_w, image_h) = image;
    if !(paint_w > 0.0 && paint_h > 0.0) {
        return None;
    }
    if image_w <= 0.0 || image_h <= 0.0 || !image_w.is_finite() || !image_h.is_finite() {
        return None;
    }
    if !pos_w.is_finite() || !pos_h.is_finite() {
        return None;
    }
    // Effective tile size after `round` rescaling on each axis. `space` and
    // `repeat` never rescale, so their effective size stays the base size.
    let (tile_w, x_count) = round_axis_tiles(pos_w, image_w, repeat.x);
    let (tile_h, y_count) = round_axis_tiles(pos_h, image_h, repeat.y);
    // cov:ignore: defensive for non-finite rescaled tiles; base and positioning are finite here
    if tile_w <= 0.0 || tile_h <= 0.0 || !tile_w.is_finite() || !tile_h.is_finite() {
        return None;
    }
    let x = axis_origins(
        pos_x,
        pos_w,
        paint_x,
        paint_x + paint_w,
        tile_w,
        image_w,
        &position.horizontal,
        &repeat.x,
        x_count,
    )?;
    let y = axis_origins(
        pos_y,
        pos_h,
        paint_y,
        paint_y + paint_h,
        tile_h,
        image_h,
        &position.vertical,
        &repeat.y,
        y_count,
    )?;
    // cov:ignore: defensive for non-finite tiles; origins are finite here
    let x: Vec<f64> = x.into_iter().filter(|value| value.is_finite()).collect();
    let y: Vec<f64> = y.into_iter().filter(|value| value.is_finite()).collect();
    if x.is_empty() || y.is_empty() {
        return None;
    }
    Some(BackgroundTiles {
        width: tile_w,
        height: tile_h,
        x,
        y,
    })
}

/// Rescaled tile size and tile count for one axis under `round`.
///
/// `round` rescales so a whole number of tiles exactly fills the positioning
/// length: `n = max(1, round(positioning / base))`, `effective = positioning / n`.
/// Other keywords keep the base size; the count is resolved later by
/// [`axis_origins`] (`space` needs the base size, `repeat` tiles to the
/// painting area). A non-positive positioning length cannot host a `round`
/// tile, so the base size is kept and the axis paints a single tile clipped to
/// the painting area.
fn round_axis_tiles(
    positioning_len: f64,
    base_len: f64,
    keyword: BackgroundRepeatKeyword,
) -> (f64, Option<i64>) {
    if !matches!(keyword, BackgroundRepeatKeyword::Round) {
        return (base_len, None);
    }
    if positioning_len.is_nan() || positioning_len <= 0.0 || base_len.is_nan() || base_len <= 0.0 {
        return (base_len, Some(1));
    }
    let count = (positioning_len / base_len).round() as i64;
    let count = count.max(1);
    // Guard against absurd counts from tiny base sizes; tiling is bounded by
    // the painting-area `repeat` path, but `round`/`space` allocate one entry
    // per tile inside `positioning`.
    // cov:ignore: defensive bound for degenerate tiny tiles; tested via empty-repeat guard
    if count > 10_000 {
        return (base_len, Some(1));
    }
    (positioning_len / count as f64, Some(count))
}

/// Tile origins for one axis.
///
/// `positioning_origin`/`positioning_len` describe the `background-origin` edge;
/// `painting_min`/`painting_max` describe the `background-clip` edge. `tile`
/// is the effective (possibly `round`-rescaled) size and `base` the
/// `background-size` size used for `space` fitting. Returns `None` when the
/// geometry cannot place a tile (non-finite positioning).
#[allow(clippy::too_many_arguments)]
fn axis_origins(
    positioning_origin: f64,
    positioning_len: f64,
    painting_min: f64,
    painting_max: f64,
    tile: f64,
    base: f64,
    offset: &ComputedCssPositionOffset,
    keyword: &BackgroundRepeatKeyword,
    round_count: Option<i64>,
) -> Option<Vec<f64>> {
    if !positioning_origin.is_finite() || !positioning_len.is_finite() {
        return None;
    }
    if !painting_min.is_finite() || !painting_max.is_finite() {
        return None;
    }
    match keyword {
        BackgroundRepeatKeyword::Repeat => {
            let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
            // cov:ignore: defensive for non-finite offsets; positioning and tile are finite here
            if !origin.is_finite() {
                return None;
            }
            // Bound the tile fan-out so a degenerate tiny tile cannot allocate
            // an unbounded origin list; the painting clip keeps the visible
            // result identical.
            let start = ((painting_min - origin) / tile).floor() as i64;
            let end = ((painting_max - origin) / tile).ceil() as i64;
            // cov:ignore: `end <= start` is defensive for empty painting (checked earlier);
            // the `> 10_000` bound is covered by the tiny-tile test below
            if end <= start || end - start > 10_000 {
                return Some(Vec::new());
            }
            Some(
                (start..end)
                    .map(|tile_index| origin + tile_index as f64 * tile)
                    .collect(),
            )
        }
        BackgroundRepeatKeyword::NoRepeat => {
            let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
            // cov:ignore: defensive for non-finite offsets; inputs are finite here
            if !origin.is_finite() {
                return None;
            }
            Some(vec![origin])
        }
        BackgroundRepeatKeyword::Space => {
            if base.is_nan() || base <= 0.0 || positioning_len.is_nan() || positioning_len <= 0.0 {
                let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
                // cov:ignore: defensive for non-finite single-tile fallback
                return origin.is_finite().then(|| vec![origin]);
            }
            let count = (positioning_len / base).floor() as i64;
            if count <= 1 {
                let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
                // cov:ignore: defensive for non-finite single-tile fallback
                return origin.is_finite().then(|| vec![origin]);
            }
            // cov:ignore: defensive bound for degenerate tiny tiles; tiny-tile test covers the repeat bound
            if count > 10_000 {
                let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
                return origin.is_finite().then(|| vec![origin]);
            }
            let gap = (positioning_len - count as f64 * base) / (count - 1) as f64;
            // cov:ignore: defensive for non-finite gaps; finite inputs give finite gaps here
            if !gap.is_finite() {
                return Some(Vec::new());
            }
            Some(
                (0..count)
                    .map(|index| positioning_origin + index as f64 * (base + gap))
                    .collect(),
            )
        }
        BackgroundRepeatKeyword::Round => {
            let count = round_count.unwrap_or(1).max(1);
            // cov:ignore: defensive bound for degenerate tiny tiles
            if count > 10_000 {
                let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
                return origin.is_finite().then(|| vec![origin]);
            }
            Some(
                (0..count)
                    .map(|index| positioning_origin + index as f64 * tile)
                    .collect(),
            )
        }
        // cov:ignore: defensive fallback for future repeat keywords
        _ => {
            let origin = positioning_origin + position_offset(*offset, positioning_len - tile);
            if !origin.is_finite() {
                return None;
            }
            Some(vec![origin])
        }
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
