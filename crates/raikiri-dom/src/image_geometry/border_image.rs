//! Used geometry of a CSS border image (CSS Backgrounds 3 §6
//! <https://www.w3.org/TR/css-backgrounds-3/#border-images>).
//!
//! [`border_image_geometry`] turns the computed `border-image-*` values and
//! the box geometry into at most nine parts. Each part names a rectangle of
//! the sized image and the tiles it is drawn into, so a painter only has to
//! draw that rectangle of the image into each tile, clipped to the part's
//! area. Painters stay free to draw raster images, vector images and
//! gradients their own way.

use raikiri_style::ComputedBorderImage;
use raikiri_style::property::{
    BorderImageOutsetSide, BorderImageRepeatKeyword, BorderImageSliceOffset, BorderImageWidthSide,
    Sides,
};
use raikiri_style::resolve::ComputedLengthPercentageOrCalc;
use raikiri_traits::ImageIntrinsicSize;

/// A rectangle as x, y, width and height, in CSS pixels.
pub type Rect = (f64, f64, f64, f64);

/// Upper bound of tiles on one axis of one part. A part that would need
/// more is drawn stretched, which looks the same at that density.
const MAX_TILES: usize = 4096;

/// Upper bound of tiles in one part, both axes together.
const MAX_PART_TILES: usize = 65_536;

/// One of the nine parts of a border image.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderImagePart {
    /// The rectangle of the sized image this part shows, in the image's
    /// own coordinates (`0..image width`, `0..image height`).
    pub source: Rect,
    /// The part of the border image area the tiles are clipped to.
    pub area: Rect,
    /// Width of every tile.
    pub tile_width: f64,
    /// Height of every tile.
    pub tile_height: f64,
    /// Left edges of the tile columns that cross the visible area.
    pub x: Vec<f64>,
    /// Top edges of the tile rows that cross the visible area.
    pub y: Vec<f64>,
}

/// The horizontal and vertical ranges of the page a painter shows, in the
/// coordinates of the border box. A tile that lies outside both ranges is
/// not part of the geometry, so a box cut into fragments does not build the
/// tiles of the other fragments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibleArea {
    /// The horizontal range, as its lowest and highest coordinate.
    pub x: (f64, f64),
    /// The vertical range, as its lowest and highest coordinate.
    pub y: (f64, f64),
}

impl VisibleArea {
    /// Everything: no tile is left out.
    pub const UNBOUNDED: Self = Self {
        x: (f64::NEG_INFINITY, f64::INFINITY),
        y: (f64::NEG_INFINITY, f64::INFINITY),
    };
}

/// The parts of one border image, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderImageGeometry {
    /// The size the image is drawn at before slicing: its natural size, or
    /// the border image area for an image without one (a gradient).
    pub image_size: (f64, f64),
    /// The border image area: the border box extended by
    /// `border-image-outset`.
    pub area: Rect,
    /// The non-empty parts: corners, edges and, with `fill`, the middle.
    pub parts: Vec<BorderImagePart>,
}

/// The largest side width kept before overlapping widths are reduced.
const MAX_SIDE_WIDTH: f64 = 1.0e30;

/// The largest outset kept, far beyond any page, so the border image area
/// stays finite.
const MAX_OUTSET: f64 = 1.0e7;

/// Compute the parts of the border image of a box (CSS Backgrounds 3 §6.6).
///
/// `border_box` is the box's border box and `border_widths` its computed
/// border widths (top, right, bottom, left), which `<number>` widths and
/// outsets multiply. `natural` is the image's natural size, `None` for an
/// image without one, such as a gradient. Returns `None` when nothing is
/// drawn.
///
/// The geometry keeps fractional CSS pixels, so moving a box moves its
/// border image without changing its shape. A raster painter that knows its
/// output scale uses [`border_image_geometry_on_device_pixels`] instead.
pub fn border_image_geometry(
    border_box: Rect,
    border_widths: Sides<f64>,
    image: &ComputedBorderImage,
    natural: Option<ImageIntrinsicSize>,
) -> Option<BorderImageGeometry> {
    geometry(
        border_box,
        border_widths,
        image,
        natural,
        None,
        VisibleArea::UNBOUNDED,
    )
}

/// A raster output's device pixel grid along each axis: the CSS pixel
/// coordinate `x` lies at device pixel `x * scale_x + offset_x`, and likewise
/// for `y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviceGrid {
    /// Device pixels per CSS pixel along x.
    pub scale_x: f64,
    /// The device pixel position of x = 0.
    pub offset_x: f64,
    /// Device pixels per CSS pixel along y.
    pub scale_y: f64,
    /// The device pixel position of y = 0.
    pub offset_y: f64,
}

impl DeviceGrid {
    /// A grid of `scale` device pixels per CSS pixel with its origin at the
    /// CSS origin.
    pub fn uniform(scale: f64) -> Self {
        Self {
            scale_x: scale,
            offset_x: 0.0,
            scale_y: scale,
            offset_y: 0.0,
        }
    }

    fn is_usable(self) -> bool {
        [self.scale_x, self.offset_x, self.scale_y, self.offset_y]
            .into_iter()
            .all(f64::is_finite)
            && self.scale_x != 0.0
            && self.scale_y != 0.0
    }
}

/// [`border_image_geometry`] for a raster output whose device pixels lie on
/// `grid`.
///
/// Opposite widths that overlap are scaled down together, which leaves the
/// inner edges of the corners at fractions of a device pixel even for
/// whole-pixel widths. Those edges move to the nearest device pixel, so
/// neighbouring parts share an edge instead of both half-covering one pixel
/// and letting the background bleed through the seam. A grid with a zero or
/// non-finite component leaves the geometry unsnapped.
pub fn border_image_geometry_on_device_pixels(
    border_box: Rect,
    border_widths: Sides<f64>,
    image: &ComputedBorderImage,
    natural: Option<ImageIntrinsicSize>,
    grid: DeviceGrid,
) -> Option<BorderImageGeometry> {
    border_image_geometry_within(
        border_box,
        border_widths,
        image,
        natural,
        Some(grid),
        VisibleArea::UNBOUNDED,
    )
}

/// [`border_image_geometry`] for a painter that shows only `visible`, on the
/// device pixels of `grid` when it has one.
///
/// The parts and the tile sizes are those of the whole border image, so
/// painting the visible tiles of every fragment of a box gives the same
/// image as painting all of them. Only the tile positions are limited: a
/// part lists the tiles that cross `visible`, which is what a painter
/// clipped to it would draw.
pub fn border_image_geometry_within(
    border_box: Rect,
    border_widths: Sides<f64>,
    image: &ComputedBorderImage,
    natural: Option<ImageIntrinsicSize>,
    grid: Option<DeviceGrid>,
    visible: VisibleArea,
) -> Option<BorderImageGeometry> {
    let grid = grid.filter(|grid| grid.is_usable());
    geometry(border_box, border_widths, image, natural, grid, visible)
}

fn geometry(
    border_box: Rect,
    border_widths: Sides<f64>,
    image: &ComputedBorderImage,
    natural: Option<ImageIntrinsicSize>,
    grid: Option<DeviceGrid>,
    visible: VisibleArea,
) -> Option<BorderImageGeometry> {
    let (x, y, width, height) = border_box;
    if ![x, y, width, height].into_iter().all(f64::is_finite) {
        return None;
    }
    // §6.4: the outset extends the border image area beyond the border box.
    let outset = |side: BorderImageOutsetSide<f32>, border: f64| {
        let outset = match side {
            BorderImageOutsetSide::Length(px) => f64::from(px),
            BorderImageOutsetSide::Number(number) => f64::from(number) * border,
            _ => 0.0,
        };
        // Overflowing values are clamped like widths below; an infinite
        // multiple of a zero border width is NaN and extends nothing.
        if outset.is_nan() {
            0.0
        } else {
            outset.clamp(0.0, MAX_OUTSET)
        }
    };
    let outsets = Sides {
        top: outset(image.outset.top, border_widths.top),
        right: outset(image.outset.right, border_widths.right),
        bottom: outset(image.outset.bottom, border_widths.bottom),
        left: outset(image.outset.left, border_widths.left),
    };
    let area = (
        x - outsets.left,
        y - outsets.top,
        width + outsets.left + outsets.right,
        height + outsets.top + outsets.bottom,
    );
    let (area_x, area_y, area_w, area_h) = area;
    if !(area_w > 0.0 && area_h > 0.0 && area_w.is_finite() && area_h.is_finite()) {
        return None;
    }

    // §6.1: the image is sized with the border image area as the default
    // object size.
    let (image_w, image_h) = default_sized(natural, area_w, area_h)?;

    // §6.2: slices are numbers in image coordinates or percentages of the
    // image size, and larger values are interpreted as 100%.
    let slice = |offset: BorderImageSliceOffset, size: f64| {
        let value = match offset {
            BorderImageSliceOffset::Number(number) => f64::from(number),
            BorderImageSliceOffset::Percent(percent) => size * f64::from(percent) / 100.0,
            _ => 0.0,
        };
        value.clamp(0.0, size)
    };
    let offsets = image.slice.offsets;
    let slices = Sides {
        top: slice(offsets.top, image_h),
        right: slice(offsets.right, image_w),
        bottom: slice(offsets.bottom, image_h),
        left: slice(offsets.left, image_w),
    };

    // §6.3: widths of the border image area's edges.
    // `auto` uses the natural height of the top and bottom slices and the
    // natural width of the left and right ones, falling back to the border
    // width when the image lacks that dimension.
    let natural_dimension =
        |dimension: Option<f32>| dimension.is_some_and(|dimension| dimension > 0.0);
    let has_natural_width = natural.is_some_and(|natural| natural_dimension(natural.width));
    let has_natural_height = natural.is_some_and(|natural| natural_dimension(natural.height));
    let side_width = |side: BorderImageWidthSide<ComputedLengthPercentageOrCalc>,
                      basis: f64,
                      border: f64,
                      slice,
                      has_natural: bool| {
        let width = match side {
            BorderImageWidthSide::LengthPercentage(ComputedLengthPercentageOrCalc::Px(px)) => {
                f64::from(px)
            }
            BorderImageWidthSide::LengthPercentage(ComputedLengthPercentageOrCalc::Percent(p)) => {
                basis * f64::from(p) / 100.0
            }
            BorderImageWidthSide::LengthPercentage(ComputedLengthPercentageOrCalc::Calc(calc)) => {
                f64::from(calc.px) + basis * f64::from(calc.percent) / 100.0
            }
            BorderImageWidthSide::Number(number) => f64::from(number) * border,
            _ if has_natural => slice,
            _ => border,
        };
        // An overflowing value such as `1e40` arrives as infinity; clamp it
        // so the proportional reduction below stays finite (CSS Values 4
        // §5.1 lets implementations clamp unsupported magnitudes). An
        // infinite multiple of a zero border width is NaN and draws nothing.
        if width.is_nan() {
            0.0
        } else {
            width.clamp(0.0, MAX_SIDE_WIDTH)
        }
    };
    let mut widths = Sides {
        top: side_width(
            image.width.top,
            area_h,
            border_widths.top,
            slices.top,
            has_natural_height,
        ),
        right: side_width(
            image.width.right,
            area_w,
            border_widths.right,
            slices.right,
            has_natural_width,
        ),
        bottom: side_width(
            image.width.bottom,
            area_h,
            border_widths.bottom,
            slices.bottom,
            has_natural_height,
        ),
        left: side_width(
            image.width.left,
            area_w,
            border_widths.left,
            slices.left,
            has_natural_width,
        ),
    };
    // Opposite widths that overlap are scaled down together (§6.3). On a
    // raster output the inner edges then move to the nearest device pixel,
    // see `border_image_geometry_on_device_pixels`. Rounding keeps the edges
    // in order, and edges that met before still meet.
    let factor = (area_w / (widths.left + widths.right))
        .min(area_h / (widths.top + widths.bottom))
        .min(1.0);
    if factor < 1.0 {
        let snap =
            |edge: f64, scale: f64, offset: f64| ((edge * scale + offset).round() - offset) / scale;
        let snap_x = |x: f64| grid.map_or(x, |grid| snap(x, grid.scale_x, grid.offset_x));
        let snap_y = |y: f64| grid.map_or(y, |grid| snap(y, grid.scale_y, grid.offset_y));
        let inner_left = snap_x(area_x + widths.left * factor);
        let inner_right = snap_x(area_x + area_w - widths.right * factor);
        let inner_top = snap_y(area_y + widths.top * factor);
        let inner_bottom = snap_y(area_y + area_h - widths.bottom * factor);
        widths = Sides {
            top: (inner_top - area_y).clamp(0.0, area_h),
            right: (area_x + area_w - inner_right).clamp(0.0, area_w),
            bottom: (area_y + area_h - inner_bottom).clamp(0.0, area_h),
            left: (inner_left - area_x).clamp(0.0, area_w),
        };
    }

    let middle_w = area_w - widths.left - widths.right;
    let middle_h = area_h - widths.top - widths.bottom;
    let source_middle_w = image_w - slices.left - slices.right;
    let source_middle_h = image_h - slices.top - slices.bottom;
    // The positions of the area's columns and rows, and of the image's.
    let columns = [
        (area_x, widths.left),
        (area_x + widths.left, middle_w),
        (area_x + area_w - widths.right, widths.right),
    ];
    let rows = [
        (area_y, widths.top),
        (area_y + widths.top, middle_h),
        (area_y + area_h - widths.bottom, widths.bottom),
    ];
    let source_columns = [
        (0.0, slices.left),
        (slices.left, source_middle_w),
        (image_w - slices.right, slices.right),
    ];
    let source_rows = [
        (0.0, slices.top),
        (slices.top, source_middle_h),
        (image_h - slices.bottom, slices.bottom),
    ];
    // The scale of each edge image: the edge width over its slice.
    let scale = |width: f64, slice: f64| {
        let scale = width / slice;
        (scale.is_finite() && scale > 0.0).then_some(scale)
    };
    let top_scale = scale(widths.top, slices.top);
    let bottom_scale = scale(widths.bottom, slices.bottom);
    let left_scale = scale(widths.left, slices.left);
    let right_scale = scale(widths.right, slices.right);

    let mut parts = Vec::with_capacity(9);
    for (row, (&(dest_y, dest_h), &(src_y, src_h))) in rows.iter().zip(&source_rows).enumerate() {
        for (column, (&(dest_x, dest_w), &(src_x, src_w))) in
            columns.iter().zip(&source_columns).enumerate()
        {
            let middle = row == 1 && column == 1;
            if middle && !image.slice.fill {
                continue;
            }
            if !(dest_w > 0.0 && dest_h > 0.0 && src_w > 0.0 && src_h > 0.0) {
                continue;
            }
            let dest = (dest_x, dest_y, dest_w, dest_h);
            let source = (src_x, src_y, src_w, src_h);
            // The size of one tile before `border-image-repeat`: corners
            // fill their area; an edge is as thick as its area and keeps
            // its slice's aspect ratio; the middle scales like the top
            // (or bottom) edge horizontally and like the left (or right)
            // edge vertically.
            let (tile_w, tile_h) = match (row, column) {
                (1, 1) => (
                    src_w * top_scale.or(bottom_scale).unwrap_or(1.0),
                    src_h * left_scale.or(right_scale).unwrap_or(1.0),
                ),
                (_, 1) => (src_w * dest_h / src_h, dest_h),
                (1, _) => (dest_w, src_h * dest_w / src_w),
                _ => (dest_w, dest_h),
            };
            let horizontal = if column == 1 {
                image.repeat.horizontal
            } else {
                BorderImageRepeatKeyword::Stretch
            };
            let vertical = if row == 1 {
                image.repeat.vertical
            } else {
                BorderImageRepeatKeyword::Stretch
            };
            let Some(columns) = tile_axis(dest_x, dest_w, tile_w, horizontal, visible.x) else {
                continue;
            };
            let Some(rows) = tile_axis(dest_y, dest_h, tile_h, vertical, visible.y) else {
                continue;
            };
            // A dense filled middle multiplies both axes; past the bound the
            // part is drawn stretched instead, whatever part of it is
            // visible.
            let (tile_width, x, tile_height, y) =
                if columns.count.saturating_mul(rows.count) > MAX_PART_TILES {
                    (
                        dest_w,
                        visible_origins(dest_x, dest_w, dest_w, 1, visible.x),
                        dest_h,
                        visible_origins(dest_y, dest_h, dest_h, 1, visible.y),
                    )
                } else {
                    (columns.tile, columns.origins, rows.tile, rows.origins)
                };
            parts.push(BorderImagePart {
                source,
                area: dest,
                tile_width,
                tile_height,
                x,
                y,
            });
        }
    }
    (!parts.is_empty()).then_some(BorderImageGeometry {
        image_size: (image_w, image_h),
        area,
        parts,
    })
}

/// The size of an image sized by the default sizing algorithm with no
/// specified size and `area_w` × `area_h` as the default object size
/// (CSS Images 3 §4.3).
fn default_sized(
    natural: Option<ImageIntrinsicSize>,
    area_w: f64,
    area_h: f64,
) -> Option<(f64, f64)> {
    let valid = |value: Option<f32>| {
        value
            .map(f64::from)
            .filter(|value| value.is_finite() && *value > 0.0)
    };
    let natural = natural.unwrap_or(ImageIntrinsicSize {
        width: None,
        height: None,
        aspect_ratio: None,
    });
    let width = valid(natural.width);
    let height = valid(natural.height);
    let ratio = valid(natural.aspect_ratio).or_else(|| width.zip(height).map(|(w, h)| w / h));
    let size = match (width, height, ratio) {
        (Some(width), Some(height), _) => (width, height),
        (Some(width), None, Some(ratio)) => (width, width / ratio),
        (None, Some(height), Some(ratio)) => (height * ratio, height),
        (Some(width), None, None) => (width, area_h),
        (None, Some(height), None) => (area_w, height),
        // Contain the ratio in the default object size.
        (None, None, Some(ratio)) if area_w / area_h > ratio => (area_h * ratio, area_h),
        (None, None, Some(ratio)) => (area_w, area_w / ratio),
        (None, None, None) => (area_w, area_h),
    };
    (size.0.is_finite() && size.1.is_finite() && size.0 > 0.0 && size.1 > 0.0).then_some(size)
}

/// The tiles of one axis of a part.
#[derive(Debug, PartialEq)]
struct TileAxis {
    /// The size of every tile.
    tile: f64,
    /// How many tiles cover the whole part, whatever is visible.
    count: usize,
    /// The origins of the tiles that cross the visible range.
    origins: Vec<f64>,
}

/// The tile size and tile origins of one axis of a part (§6.6 steps 2
/// and 3): `start` and `length` are the part's extent and `tile` the tile
/// size before `keyword` applies. Only the tiles crossing `visible` get an
/// origin.
fn tile_axis(
    start: f64,
    length: f64,
    tile: f64,
    keyword: BorderImageRepeatKeyword,
    visible: (f64, f64),
) -> Option<TileAxis> {
    if !(tile > 0.0 && tile.is_finite()) {
        return None;
    }
    let count = (length / tile).ceil();
    let keyword = if count > MAX_TILES as f64 {
        BorderImageRepeatKeyword::Stretch
    } else {
        keyword
    };
    let axis = |tile: f64, first: f64, stride: f64, count: usize| TileAxis {
        tile,
        count,
        origins: visible_origins(first, stride, tile, count, visible),
    };
    match keyword {
        // Centered, and repeated outwards to cover the part.
        BorderImageRepeatKeyword::Repeat => {
            let first = start + (length - tile) / 2.0;
            let before = ((first - start) / tile).ceil().max(0.0);
            let first = first - before * tile;
            let count = ((start + length - first) / tile).ceil() as usize;
            Some(axis(tile, first, tile, count))
        }
        // A whole number of tiles, resized to fill the part.
        BorderImageRepeatKeyword::Round => {
            let count = (length / tile).round().max(1.0);
            let tile = length / count;
            Some(axis(tile, start, tile, count as usize))
        }
        // As many whole tiles as fit, with equal space around them.
        BorderImageRepeatKeyword::Space => {
            let count = (length / tile).floor();
            if count < 1.0 {
                return None;
            }
            let gap = (length - count * tile) / (count + 1.0);
            Some(axis(tile, start + gap, tile + gap, count as usize))
        }
        // `stretch`: one tile as large as the part.
        _ => Some(axis(length, start, length, 1)),
    }
}

/// The origins `first + i * stride` of the `count` tiles of width `tile`
/// that cross `visible`, in order.
///
/// A tile crosses the range when it starts at or before its upper end and
/// ends at or after its lower end. The indices that can cross are bracketed
/// by arithmetic, widened by one on each side to absorb rounding, and each
/// candidate is then checked exactly, so the result is what filtering all
/// `count` tiles gives without building the others.
fn visible_origins(
    first: f64,
    stride: f64,
    tile: f64,
    count: usize,
    visible: (f64, f64),
) -> Vec<f64> {
    let low = ((visible.0 - tile - first) / stride).floor() - 1.0;
    let high = ((visible.1 - first) / stride).ceil() + 1.0;
    // `as usize` saturates, and `max`/`min` ignore a NaN bound.
    let low = low.max(0.0) as usize;
    let high = high.min(count.saturating_sub(1) as f64) as usize;
    (low..=high)
        .map(|i| first + i as f64 * stride)
        .filter(|origin| *origin <= visible.1 && *origin + tile >= visible.0)
        .collect()
}

#[cfg(test)]
mod tests;
