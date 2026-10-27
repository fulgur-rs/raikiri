//! Linear and radial gradient images (CSS Images 3 §3.1–§3.6).
//!
//! A gradient has no natural size, so it fills each background tile it is
//! sized to. The brush is built once for a tile at the origin and moved onto
//! every tile with a brush transform.

use super::*;
use raikiri_dom::image_geometry::BackgroundTiles;
use raikiri_style::property::{
    BackgroundRepeat, BackgroundRepeatKeyword, GradientColorInterpolation, GradientColorStop,
    HorizontalSide, LengthPercentageOrCalc, LinearGradient, LinearGradientDirection, RadialExtent,
    RadialGradient, RadialShape, RadialSize, VerticalSide,
};

/// What one gradient tile is filled with.
#[derive(Debug, Clone)]
pub(super) enum GradientFill {
    /// A gradient brush for a tile at the origin, with an optional brush
    /// transform that maps the circle of an elliptical radial gradient onto
    /// its ellipse.
    Brush(Box<PenikoGradient>, Option<Affine>),
    /// A repeating gradient whose stops share one position paints the
    /// average of its stop colors (CSS Images 3 §3.6).
    Solid(Color),
}

/// Used size of a gradient tile (CSS Backgrounds 3 §3.9).
///
/// A gradient has no natural dimensions, so `auto`, `contain` and `cover`
/// all take the size of the positioning area on that axis.
pub(super) fn gradient_image_size(size: &ComputedBackgroundSize, area: Rect) -> Option<(f64, f64)> {
    let (area_w, area_h) = (area.width(), area.height());
    let resolve = |value: ComputedLengthPercentageOrAuto, basis: f64| match value {
        ComputedLengthPercentageOrAuto::Px(px) => f64::from(px),
        ComputedLengthPercentageOrAuto::Percent(percent) => basis * f64::from(percent) / 100.0,
        ComputedLengthPercentageOrAuto::Calc(calc) => {
            basis * f64::from(calc.percent) / 100.0 + f64::from(calc.px)
        }
        _ => basis,
    };
    let (width, height) = match size {
        ComputedBackgroundSize::Explicit { width, height } => {
            (resolve(*width, area_w), resolve(*height, area_h))
        }
        _ => (area_w, area_h),
    };
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then_some((width, height))
}

/// Paint a linear or radial gradient layer.
///
/// `positioning` is the `background-origin` box that sizes and places the
/// tiles; `painting` is the `background-clip` box the tiles cover and are
/// clipped to. A box with rounded corners is clipped to its curve by the
/// caller, around the background color and the gradient together. Returns
/// false for a gradient this module does not draw (a conic gradient), so the
/// caller can fall back to its own path.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_gradient_layer(
    scene: &mut impl PaintScene,
    gradient: &Gradient,
    positioning: Rect,
    painting: Rect,
    size: &ComputedBackgroundSize,
    position: &ComputedCssPosition,
    repeat: &raikiri_style::property::BackgroundRepeat,
    current_color: CssColor,
) -> bool {
    if !matches!(gradient, Gradient::Linear(_) | Gradient::Radial(_)) {
        return false;
    }
    GradientLayer {
        gradient,
        positioning,
        painting,
        size,
        position,
        repeat,
        current_color,
    }
    .paint(scene);
    true
}

/// One gradient layer: the boxes it is placed in and how its tiles repeat.
struct GradientLayer<'a> {
    gradient: &'a Gradient,
    /// The `background-origin` box.
    positioning: Rect,
    /// The `background-clip` box.
    painting: Rect,
    size: &'a ComputedBackgroundSize,
    position: &'a ComputedCssPosition,
    repeat: &'a BackgroundRepeat,
    current_color: CssColor,
}

/// The used size of the tiles of a layer and whether each axis repeats.
#[derive(Clone, Copy)]
struct TileSizes {
    width: f64,
    height: f64,
    repeat_x: bool,
    repeat_y: bool,
}

impl TileSizes {
    /// How many tiles at most cover a painting area of the given size, with
    /// two spare tiles on a repeating axis.
    fn estimated_count(&self, painting: Rect) -> f64 {
        let estimate = |repeats: bool, extent: f64, tile: f64| {
            if repeats {
                (extent / tile).ceil() + 2.0
            } else {
                1.0
            }
        };
        estimate(self.repeat_x, painting.width(), self.width)
            * estimate(self.repeat_y, painting.height(), self.height)
    }
}

impl GradientLayer<'_> {
    /// Paints the layer, unless its tiles have no size.
    fn paint(&self, scene: &mut impl PaintScene) {
        if let Some(sizes) = self.tile_sizes() {
            self.paint_sized(scene, sizes);
        }
    }

    /// The size of the tiles and whether each axis repeats.
    fn tile_sizes(&self) -> Option<TileSizes> {
        gradient_image_size(self.size, self.positioning).map(|(tile_w, tile_h)| {
            let (width, height) = (
                enlarged(self.repeat.x, tile_w),
                enlarged(self.repeat.y, tile_h),
            );
            TileSizes {
                width,
                height,
                repeat_x: axis_repeats(self.repeat.x, self.positioning.width(), width),
                repeat_y: axis_repeats(self.repeat.y, self.positioning.height(), height),
            }
        })
    }

    /// Paints the tiles of the given size, one by one or, when there are too
    /// many of them, as the average color of one tile.
    fn paint_sized(&self, scene: &mut impl PaintScene, sizes: TileSizes) {
        // Too many tiles to fill one by one, such as one-pixel tiles over a
        // large box, are approximated by the average color of one tile, which
        // keeps the scene bounded.
        if sizes.estimated_count(self.painting) > MAX_GRADIENT_TILES as f64 {
            self.paint_average(scene, sizes);
        } else {
            self.paint_tiles(scene, sizes);
        }
    }

    /// Fills the area the tiles would cover with the average color of one
    /// tile.
    fn paint_average(&self, scene: &mut impl PaintScene, sizes: TileSizes) {
        let positioning = self.positioning;
        // A `no-repeat` axis still shows its one positioned tile.
        let span = |repeats: bool, start: f64, extent: f64, tile: f64, offset| {
            if repeats {
                (f64::NEG_INFINITY, f64::INFINITY)
            } else {
                let start = start + position_offset(offset, extent - tile);
                (start, start + tile)
            }
        };
        let (x0, x1) = span(
            sizes.repeat_x,
            positioning.x0,
            positioning.width(),
            sizes.width,
            self.position.horizontal,
        );
        let (y0, y1) = span(
            sizes.repeat_y,
            positioning.y0,
            positioning.height(),
            sizes.height,
            self.position.vertical,
        );
        let painting = self.painting;
        let area = Rect::new(
            x0.max(painting.x0),
            y0.max(painting.y0),
            x1.min(painting.x1),
            y1.min(painting.y1),
        );
        let tile = Rect::new(0.0, 0.0, sizes.width, sizes.height);
        if area.width() > 0.0
            && area.height() > 0.0
            && let Some(fill) = gradient_fill(self.gradient, tile, self.current_color)
        {
            fill_average(scene, &fill, area);
        }
    }

    /// Fills each tile of the layer, clipped to the painting area.
    fn paint_tiles(&self, scene: &mut impl PaintScene, sizes: TileSizes) {
        let positioning = self.positioning;
        let painting = self.painting;
        let Some(tiles) = background_tiles(
            (
                positioning.x0,
                positioning.y0,
                positioning.width(),
                positioning.height(),
            ),
            (
                painting.x0,
                painting.y0,
                painting.width(),
                painting.height(),
            ),
            (sizes.width, sizes.height),
            self.position,
            self.repeat,
        ) else {
            return;
        };
        // `round` may rescale the tile, so the brush is built for the used size.
        let tile = Rect::new(0.0, 0.0, tiles.width, tiles.height);
        if let Some(fill) = gradient_fill(self.gradient, tile, self.current_color) {
            self.fill_tiles(scene, &fill, &tiles, sizes);
        }
    }

    /// Fills every tile with `fill`, clipped to the painting area, or the
    /// whole area with its average color when there are too many tiles.
    fn fill_tiles(
        &self,
        scene: &mut impl PaintScene,
        fill: &GradientFill,
        tiles: &BackgroundTiles,
        sizes: TileSizes,
    ) {
        if tiles.x.len().saturating_mul(tiles.y.len()) > MAX_GRADIENT_TILES {
            fill_average(scene, fill, self.painting);
            return;
        }
        scene.push_clip_layer(Affine::IDENTITY, &self.painting);
        for tile_y in &tiles.y {
            for tile_x in &tiles.x {
                self.fill_tile(scene, fill, tiles, (*tile_x, *tile_y), sizes);
            }
        }
        scene.pop_layer();
    }

    /// Fills the tile at (`tile_x`, `tile_y`) with the gradient.
    fn fill_tile(
        &self,
        scene: &mut impl PaintScene,
        fill: &GradientFill,
        tiles: &BackgroundTiles,
        (tile_x, tile_y): (f64, f64),
        sizes: TileSizes,
    ) {
        // Repeated tile edges snap to whole pixels so neighboring tiles
        // meet without antialiased seams; a single tile and the
        // painting area keep their own edges.
        let (snap_x, snap_y) = (
            sizes.repeat_x && tiles.x.len() > 1,
            sizes.repeat_y && tiles.y.len() > 1,
        );
        let snapped = Rect::new(
            snapped_edge(tile_x, snap_x),
            snapped_edge(tile_y, snap_y),
            snapped_edge(tile_x + tiles.width, snap_x),
            snapped_edge(tile_y + tiles.height, snap_y),
        );
        let shape = snapped.intersect(self.painting);
        if shape.width() <= 0.0 || shape.height() <= 0.0 {
            return;
        }
        // The brush is stretched onto the snapped tile, so every copy
        // shows the whole gradient from its first color to its last.
        let offset = Affine::translate((snapped.x0, snapped.y0))
            * Affine::scale_non_uniform(
                snapped.width() / tiles.width,
                snapped.height() / tiles.height,
            );
        match fill {
            GradientFill::Brush(brush, brush_transform) => scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                brush.as_ref(),
                Some(offset * brush_transform.unwrap_or(Affine::IDENTITY)),
                &shape,
            ),
            GradientFill::Solid(color) => {
                scene.fill(Fill::NonZero, Affine::IDENTITY, *color, None, &shape)
            }
        }
    }
}

/// The tile extent of a repeat keyword. Repeated tiles below one pixel cannot
/// be drawn without coverage gaps; like browsers, repeat such a gradient in
/// one-pixel tiles. A `no-repeat` tile keeps its size.
fn enlarged(keyword: BackgroundRepeatKeyword, tile: f64) -> f64 {
    match keyword {
        BackgroundRepeatKeyword::NoRepeat => tile,
        _ => tile.max(1.0),
    }
}

/// Whether an axis of `extent` repeats tiles of the given size. `space` with
/// room for fewer than two of the used tiles places one, like `no-repeat`
/// (CSS Backgrounds 3 §2.4).
fn axis_repeats(keyword: BackgroundRepeatKeyword, extent: f64, tile: f64) -> bool {
    match keyword {
        BackgroundRepeatKeyword::NoRepeat => false,
        BackgroundRepeatKeyword::Space => extent >= 2.0 * tile,
        _ => true,
    }
}

/// A tile edge, rounded to a whole pixel when `snaps`.
fn snapped_edge(edge: f64, snaps: bool) -> f64 {
    if snaps { edge.round() } else { edge }
}

/// Fill the painting area once with the average color of a tile.
fn fill_average(scene: &mut impl PaintScene, fill: &GradientFill, painting: Rect) {
    let color = match fill {
        GradientFill::Brush(brush, _) => brush_average_color(brush),
        GradientFill::Solid(color) => *color,
    };
    scene.push_clip_layer(Affine::IDENTITY, &painting);
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &painting);
    scene.pop_layer();
}

/// Shortest repeating-gradient period, in CSS pixels, painted as a pattern:
/// under a tenth of a device pixel even at 600 dpi.
const MIN_REPEATING_PERIOD: f64 = 1.0 / 64.0;

/// Most gradient tiles one background layer fills one by one.
const MAX_GRADIENT_TILES: usize = 16_384;

/// The average color along a gradient brush, weighting each pair of stops by
/// the distance between them.
fn brush_average_color(brush: &PenikoGradient) -> Color {
    let mut stops: Vec<(f64, DynamicColor)> = brush
        .stops
        .iter()
        .map(|stop| (f64::from(stop.offset), stop.color))
        .collect();
    // A padded brush shows its first and last colors before and after its
    // stops.
    if matches!(brush.extend, PenikoExtend::Pad) {
        if let Some(&(offset, color)) = stops.first()
            && offset > 0.0
        {
            stops.insert(0, (0.0, color));
        }
        if let Some(&(offset, color)) = stops.last()
            && offset < 1.0
        {
            stops.push((1.0, color));
        }
    }
    average_color(&stops)
}

/// The fill of one linear or radial gradient tile placed at the origin.
///
/// Returns `None` for a conic gradient and for a gradient with nothing to
/// paint, such as one without color stops.
pub(super) fn gradient_fill(
    gradient: &Gradient,
    tile: Rect,
    current_color: CssColor,
) -> Option<GradientFill> {
    match gradient {
        Gradient::Linear(linear) => linear_fill(linear, tile, current_color),
        Gradient::Radial(radial) => radial_fill(radial, tile, current_color),
        _ => None,
    }
}

/// Unit direction of the gradient line in a y-down space (CSS Images 3
/// §3.1.1).
///
/// An angle counts clockwise from "to top". A corner keyword points the line
/// so that the 50% line runs through the two neighboring corners, which makes
/// it perpendicular to the diagonal between them.
fn linear_direction(direction: LinearGradientDirection, width: f64, height: f64) -> Vec2 {
    match direction {
        LinearGradientDirection::Angle(angle) => {
            let degrees = if angle.0.is_finite() { angle.0 } else { 180.0 };
            let radians = f64::from(degrees).to_radians();
            Vec2::new(radians.sin(), -radians.cos())
        }
        LinearGradientDirection::Side(side) => {
            let sx = match side.horizontal {
                Some(HorizontalSide::Left) => -1.0,
                Some(HorizontalSide::Right) => 1.0,
                _ => 0.0,
            };
            let sy = match side.vertical {
                Some(VerticalSide::Top) => -1.0,
                Some(VerticalSide::Bottom) => 1.0,
                _ => 0.0,
            };
            let corner = Vec2::new(sx * height, sy * width);
            if sx != 0.0 && sy != 0.0 && corner.hypot() > 0.0 {
                corner / corner.hypot()
            } else {
                let fallback = Vec2::new(sx, sy);
                if fallback.hypot() > 0.0 {
                    fallback / fallback.hypot()
                } else {
                    // cov:ignore: the parser requires at least one side keyword
                    Vec2::new(0.0, 1.0)
                }
            }
        }
        // cov:ignore: defensive fallback for future direction variants
        _ => Vec2::new(0.0, 1.0),
    }
}

fn linear_fill(
    linear: &LinearGradient,
    tile: Rect,
    current_color: CssColor,
) -> Option<GradientFill> {
    let (width, height) = (tile.width(), tile.height());
    let direction = linear_direction(linear.direction, width, height);
    let length = (width * direction.x).abs() + (height * direction.y).abs();
    let center = tile.center();
    let start = center - direction * (length / 2.0);
    let line = direction * length;
    let stops = resolved_stops(
        &linear.stops,
        length,
        length,
        current_color,
        &linear.interpolation,
    );
    let (first, last, stops) =
        match normalize_stops(stops, linear.repeating, length, &linear.interpolation)? {
            Normalized::Solid(color) => return Some(GradientFill::Solid(color)),
            Normalized::Stops { first, last, stops } => (first, last, stops),
        };
    let brush = PenikoGradient::new_linear(start + line * first, start + line * last);
    Some(GradientFill::Brush(
        Box::new(finish_brush(
            brush,
            &stops,
            linear.repeating,
            linear.interpolation.color_space,
            linear.interpolation.hue_method,
        )),
        None,
    ))
}

/// Horizontal and vertical radii of a radial gradient's ending shape (CSS
/// Images 3 §3.2.1).
pub(super) fn radial_radii(radial: &RadialGradient, center: Point, tile: Rect) -> (f64, f64) {
    let left = (center.x - tile.x0).abs();
    let right = (tile.x1 - center.x).abs();
    let top = (center.y - tile.y0).abs();
    let bottom = (tile.y1 - center.y).abs();
    let (near_x, far_x) = (left.min(right), left.max(right));
    let (near_y, far_y) = (top.min(bottom), top.max(bottom));
    let circle = matches!(radial.shape, RadialShape::Circle);
    // An ellipse sized to a corner keeps the aspect ratio of the matching
    // side extent and passes through that corner.
    let through_corner = |side_x: f64, side_y: f64, corner_x: f64, corner_y: f64| {
        if side_x <= 0.0 || side_y <= 0.0 {
            return (0.0, 0.0);
        }
        let ratio = side_x / side_y;
        let rx = (corner_x * corner_x + (corner_y * ratio) * (corner_y * ratio)).sqrt();
        (rx, rx / ratio)
    };
    match radial.size {
        RadialSize::Extent(RadialExtent::ClosestSide) if circle => {
            let r = near_x.min(near_y);
            (r, r)
        }
        RadialSize::Extent(RadialExtent::FarthestSide) if circle => {
            let r = far_x.max(far_y);
            (r, r)
        }
        RadialSize::Extent(RadialExtent::ClosestCorner) if circle => {
            let r = near_x.hypot(near_y);
            (r, r)
        }
        RadialSize::Extent(RadialExtent::FarthestCorner) if circle => {
            let r = far_x.hypot(far_y);
            (r, r)
        }
        RadialSize::Extent(RadialExtent::ClosestSide) => (near_x, near_y),
        RadialSize::Extent(RadialExtent::FarthestSide) => (far_x, far_y),
        RadialSize::Extent(RadialExtent::ClosestCorner) => {
            through_corner(near_x, near_y, near_x, near_y)
        }
        RadialSize::Extent(RadialExtent::FarthestCorner) => {
            through_corner(far_x, far_y, far_x, far_y)
        }
        RadialSize::Circle(length) => {
            let r = radial_radius(length, 0.0);
            (r, r)
        }
        RadialSize::Ellipse(rx, ry) => (
            radial_radius(rx, tile.width()),
            radial_radius(ry, tile.height()),
        ),
        // cov:ignore: defensive fallback for future size variants
        _ => through_corner(far_x, far_y, far_x, far_y),
    }
}

/// An explicit radius against its percentage basis. A `calc()` radius that
/// resolves below zero clamps to zero (CSS Values 4 "Range Checking",
/// <https://www.w3.org/TR/css-values-4/#calc-range>).
fn radial_radius(radius: LengthPercentageOrCalc, basis: f64) -> f64 {
    match radius {
        LengthPercentageOrCalc::Length(length) => {
            f64::from(length_to_px(length, basis as f32, DEFAULT_FONT_SIZE))
        }
        LengthPercentageOrCalc::Calc(calc) => f64::from(
            raikiri_style::resolve::ComputedLengthPercentageOrCalc::Calc(calc)
                .resolve(basis as f32),
        ),
        // The cascade absolutizes font-relative and viewport terms.
        _ => 0.0, // cov:ignore: a computed radius has no relative terms left
    }
}

fn radial_fill(
    radial: &RadialGradient,
    tile: Rect,
    current_color: CssColor,
) -> Option<GradientFill> {
    let center = conic_center(&radial.position, tile);
    let (rx, ry) = radial_radii(radial, center, tile);
    if !(rx >= 0.0 && ry >= 0.0 && rx.is_finite() && ry.is_finite()) {
        return None;
    }
    // A degenerate ending shape is drawn as a vanishingly thin one (CSS
    // Images 3 §3.2.3): percentages of its ray resolve to zero and lengths
    // keep their size, so the ray is measured here in pixels.
    let circle = matches!(radial.shape, RadialShape::Circle);
    let (rx, ry, percent_basis) = if rx > 0.0 && ry > 0.0 {
        (rx, ry, rx)
    } else if circle || rx <= 0.0 {
        // A zero-width ellipse is as tall as any distance in the tile, which
        // makes a horizontal gradient mirrored across the center.
        let ry = if circle {
            1.0
        } else {
            DEGENERATE_ELLIPSE_RATIO * tile.width().max(tile.height()).max(1.0)
        };
        (1.0, ry, 0.0)
    } else {
        // A zero-height ellipse is as wide as any distance in the tile, so
        // only its last color, or the average of a repeating one, shows.
        let stops = resolved_stops(&radial.stops, rx, rx, current_color, &radial.interpolation);
        let color = if radial.repeating {
            ramp_average_color(&stops, &radial.interpolation)
        } else {
            let (_, last) = *stops.last()?;
            solid_color(last)
        };
        return Some(GradientFill::Solid(color));
    };
    let mut stops = resolved_stops(
        &radial.stops,
        rx,
        percent_basis,
        current_color,
        &radial.interpolation,
    );
    if radial.repeating {
        // The pattern is periodic, so shifting it by whole periods keeps every
        // radius at or above zero without changing the colors.
        let first = stops.first().map_or(0.0, |stop| stop.0);
        let span = stops.last().map_or(0.0, |stop| stop.0) - first;
        if first < 0.0 && span > 0.0 {
            let shift = (-first / span).ceil() * span;
            for stop in &mut stops {
                stop.0 += shift;
            }
        }
    } else if let Some(color) = clip_stops_at_zero(&mut stops, &radial.interpolation) {
        return Some(GradientFill::Solid(color));
    }
    // An ellipse stretches the period along its longer radius, where it is
    // most visible.
    let (first, last, stops) =
        match normalize_stops(stops, radial.repeating, rx.max(ry), &radial.interpolation)? {
            Normalized::Solid(color) => return Some(GradientFill::Solid(color)),
            Normalized::Stops { first, last, stops } => (first, last, stops),
        };
    let brush = PenikoGradient::new_two_point_radial(
        center,
        (rx * first) as f32,
        center,
        (rx * last) as f32,
    );
    let ellipse = (ry != rx).then(|| {
        Affine::translate(center.to_vec2())
            * Affine::scale_non_uniform(1.0, ry / rx)
            * Affine::translate(-center.to_vec2())
    });
    Some(GradientFill::Brush(
        Box::new(finish_brush(
            brush,
            &stops,
            radial.repeating,
            radial.interpolation.color_space,
            radial.interpolation.hue_method,
        )),
        ellipse,
    ))
}

/// Font size assumed for font-relative stop positions and radii, which the
/// computed value keeps unresolved.
const DEFAULT_FONT_SIZE: f32 = 16.0;

/// How many times taller than the tile a zero-width ellipse is drawn, so
/// that vertical distances inside the tile do not change its colors.
const DEGENERATE_ELLIPSE_RATIO: f64 = 1e4;

/// A length-percentage stop position or transition hint as a fraction of the
/// gradient line or ray.
///
/// `length` is the drawn line or ray and `percent_basis` the length its
/// percentages resolve against, which differ only for a degenerate radial
/// gradient.
fn position_fraction(position: LengthPercentageOrCalc, length: f64, percent_basis: f64) -> f32 {
    // The cascade has resolved every other unit to pixels.
    let percent = match position {
        LengthPercentageOrCalc::Length(Length::Percent(percent)) => f64::from(percent),
        LengthPercentageOrCalc::Calc(calc) => f64::from(calc.percent),
        _ => 0.0,
    };
    let px = match position {
        LengthPercentageOrCalc::Length(Length::Px(px)) => f64::from(px),
        LengthPercentageOrCalc::Calc(calc) => f64::from(calc.px),
        _ => 0.0,
    };
    let fraction = if length > 0.0 {
        (px + percent / 100.0 * percent_basis) / length
    } else {
        percent / 100.0
    };
    Some(fraction)
        .filter(|fraction| fraction.is_finite())
        .map_or(0.0, |fraction| fraction as f32)
}

/// Stop positions as fractions of the gradient line or ray, after the color
/// stop fixup (CSS Images 3 §3.4.3), with each transition hint replaced by
/// the stops that draw its curve (see [`expand_hint`]).
///
/// `length` is the drawn line or ray and `percent_basis` the length its
/// percentages resolve against, which differ only for a degenerate radial
/// gradient.
fn resolved_stops(
    stops: &[GradientColorStop],
    length: f64,
    percent_basis: f64,
    current_color: CssColor,
    interpolation: &GradientColorInterpolation,
) -> Vec<(f64, DynamicColor)> {
    let mut raw: Vec<RawStop> = stops
        .iter()
        .map(|stop| RawStop {
            position: stop
                .position
                .map(|position| position_fraction(position, length, percent_basis)),
            hint: stop
                .hint
                .map(|hint| position_fraction(hint, length, percent_basis)),
            color: gradient_stop_dynamic(stop.color, current_color),
        })
        .collect();
    // An unpositioned last stop sits at 100%, which a degenerate radial
    // gradient also resolves to zero.
    if let Some(last) = raw.last_mut()
        && last.position.is_none()
        && length > 0.0
    {
        last.position = Some((percent_basis / length) as f32);
    }
    if let Some(first) = raw.first_mut()
        && first.position.is_none()
    {
        first.position = Some(0.0);
    }
    fixed_up_stops(&raw, interpolation)
}

/// A color stop as authored, with the transition hint written before it. A
/// position of `None` is filled in by [`fixed_up_stops`].
pub(super) struct RawStop {
    pub(super) position: Option<f32>,
    pub(super) hint: Option<f32>,
    pub(super) color: DynamicColor,
}

/// The offsets of authored stops after the color stop fixup (CSS Images 3
/// §3.4.3), with each transition hint replaced by the stops that draw its
/// curve.
///
/// A stop or hint that lies before an earlier one is moved up to it, then
/// the stops without a position are spread evenly between the stops and hints
/// around them, as browsers do: a hint always has a position, so it anchors
/// the run of stops next to it.
pub(super) fn fixed_up_stops(
    raw: &[RawStop],
    interpolation: &GradientColorInterpolation,
) -> Vec<(f64, DynamicColor)> {
    // One list in authored order: each hint, if any, then its stop.
    let mut positions: Vec<Option<f32>> = Vec::with_capacity(raw.len() * 2);
    let mut slots: Vec<(Option<usize>, usize)> = Vec::with_capacity(raw.len());
    for stop in raw {
        let hint = stop.hint.map(|hint| {
            positions.push(Some(hint));
            positions.len() - 1
        });
        positions.push(stop.position);
        slots.push((hint, positions.len() - 1));
    }
    let offsets = fixup_conic_offsets(&positions);
    let mut fixed: Vec<(f64, DynamicColor)> = Vec::with_capacity(raw.len());
    for (stop, (hint, at)) in raw.iter().zip(slots) {
        let offset = f64::from(offsets[at]);
        if let (Some(hint), Some(&before)) = (hint, fixed.last()) {
            // A stop moved up after the hint was placed can leave the hint
            // outside the pair it sits between.
            let hint = f64::from(offsets[hint]).clamp(before.0, offset.max(before.0));
            expand_hint(
                before,
                (offset, stop.color),
                hint,
                interpolation,
                &mut fixed,
            );
        }
        fixed.push((offset, stop.color));
    }
    fixed
}

/// Number of samples in each of the two grids that sample a hint's curve.
const HINT_SAMPLES: usize = 16;

/// How far from the middle a hint may lie and still draw a straight line.
const HINT_MIDDLE_TOLERANCE: f64 = 1e-6;

/// Push the stops that draw the color transition hint between two stops
/// (CSS Images 3 §3.4.2): the color at the fraction `t` of the way from
/// `before` to `after` is the mix of the two at `t` raised to
/// `ln(1/2) / ln(h)`, where `h` is the hint's fraction of the way, so the
/// mix is even at the hint. A hint in the middle draws the plain ramp. One
/// at either end holds a color up to the hint and jumps to the other there.
///
/// The curve is sampled at the multiples of 1/[`HINT_SAMPLES`] along the
/// stops and at the same multiples along the mix, which finds the places it
/// bends sharply, near whichever end the hint lies closer to.
///
/// The samples are joined by ramps again when the brush is built, and a ramp
/// between two neighboring samples takes the hue the long way round under
/// `longer hue`, so that method draws the plain ramp and leaves the hint out.
/// The other hue methods follow the samples.
fn expand_hint(
    before: (f64, DynamicColor),
    after: (f64, DynamicColor),
    hint: f64,
    interpolation: &GradientColorInterpolation,
    out: &mut Vec<(f64, DynamicColor)>,
) {
    let span = after.0 - before.0;
    if span <= 0.0 || interpolation.hue_method == HueInterpolationMethod::Longer {
        return;
    }
    let h = ((hint - before.0) / span).clamp(0.0, 1.0);
    if (h - 0.5).abs() < HINT_MIDDLE_TOLERANCE {
        return;
    }
    if h <= 0.0 {
        out.push((before.0, after.1));
        return;
    }
    if h >= 1.0 {
        out.push((after.0, before.1));
        return;
    }
    let exponent = 0.5f64.ln() / h.ln();
    let mut fractions: Vec<f64> = (1..HINT_SAMPLES)
        .flat_map(|step| {
            let t = step as f64 / HINT_SAMPLES as f64;
            [t, t.powf(exponent.recip())]
        })
        .collect();
    fractions.sort_by(f64::total_cmp);
    fractions.dedup();
    let ramp = before.1.interpolate(
        after.1,
        conic_interpolation_tag(interpolation.color_space),
        conic_hue_direction(interpolation.hue_method),
    );
    for t in fractions {
        // A hint very close to a stop packs its curve against that stop in a
        // range as narrow as the float precision allows.
        let offset = before.0 + span * t;
        if offset > before.0 && offset < after.0 {
            out.push((offset, ramp.eval(t.powf(exponent) as f32)));
        }
    }
}

/// Drop stops below zero from a radial gradient, keeping the color the line
/// has at zero.
///
/// Returns the solid color the gradient paints when every stop lies before
/// the center.
fn clip_stops_at_zero(
    stops: &mut Vec<(f64, DynamicColor)>,
    interpolation: &GradientColorInterpolation,
) -> Option<Color> {
    let Some(index) = stops.iter().position(|stop| stop.0 >= 0.0) else {
        let &(_, last) = stops.last()?;
        return Some(solid_color(last));
    };
    if index == 0 {
        return None;
    }
    let (before_offset, before) = stops[index - 1];
    let (after_offset, after) = stops[index];
    let t = if after_offset > before_offset {
        -before_offset / (after_offset - before_offset)
    } else {
        // cov:ignore: fixup keeps offsets non-decreasing, so a gap below zero is positive
        1.0
    };
    let color = lerp_color(before, after, t, interpolation);
    stops.drain(..index);
    stops.insert(0, (0.0, color));
    None
}

/// The color `t` of the way from `from` to `to` in the gradient's
/// interpolation space, with premultiplied alpha (CSS Images 4 §3.5.2).
fn lerp_color(
    from: DynamicColor,
    to: DynamicColor,
    t: f64,
    interpolation: &GradientColorInterpolation,
) -> DynamicColor {
    from.interpolate(
        to,
        conic_interpolation_tag(interpolation.color_space),
        conic_hue_direction(interpolation.hue_method),
    )
    .eval(t as f32)
}

/// A dynamic color drawn as one solid 8-bit sRGB color.
fn solid_color(color: DynamicColor) -> Color {
    let CssColor { r, g, b, a } = dynamic_to_css_color(color);
    Color::from_rgba8(r, g, b, a)
}

/// The average color of one period of stops (CSS Images 4 §3.5.3): each pair
/// of neighboring stops contributes the mean of its two colors, weighted by
/// the distance between them, in premultiplied sRGB. Stops that share one
/// position are spread evenly, so each pair weighs the same.
pub(super) fn average_color(stops: &[(f64, DynamicColor)]) -> Color {
    let premultiplied: Vec<[f64; 4]> = stops
        .iter()
        .map(|(_, color)| premultiply(dynamic_to_css_color(*color)))
        .collect();
    let period = match stops {
        [first, .., last] => last.0 - first.0,
        _ => 0.0,
    };
    let weights: Vec<f64> = stops
        .windows(2)
        .map(|pair| {
            if period > 1e-6 {
                (pair[1].0 - pair[0].0) / period
            } else {
                1.0 / (stops.len() - 1) as f64
            }
        })
        .collect();
    let [r, g, b, a] = match premultiplied.as_slice() {
        [] => return Color::TRANSPARENT,
        [only] => unpremultiply(*only),
        many => unpremultiply(std::array::from_fn(|channel| {
            many.windows(2)
                .zip(&weights)
                .map(|(pair, weight)| (pair[0][channel] + pair[1][channel]) / 2.0 * weight)
                .sum::<f64>()
        })),
    };
    Color::from_rgba8(r, g, b, a)
}

/// The average color of one period of stops, as [`average_color`] takes it.
/// Clipping does not commute with interpolation, so when a stop lies outside
/// the sRGB gamut the ramp is interpolated in its own space and clipped
/// before it is averaged, not the other way round. Stops that share one
/// position are spread evenly, as [`average_color`] weighs them.
fn ramp_average_color(
    stops: &[(f64, DynamicColor)],
    interpolation: &GradientColorInterpolation,
) -> Color {
    let in_gamut_stops = stops
        .iter()
        .all(|(_, color)| in_gamut(color.to_alpha_color::<Srgb>()));
    let ([first, .., last], false) = (stops, in_gamut_stops) else {
        return average_color(stops);
    };
    let period = last.0 - first.0;
    let last_index = (stops.len() - 1) as f64;
    let spread: Vec<(f32, DynamicColor)> = stops
        .iter()
        .enumerate()
        .map(|(index, &(offset, color))| {
            let offset = if period > 1e-6 {
                (offset - first.0) / period
            } else {
                index as f64 / last_index
            };
            (offset as f32, color)
        })
        .collect();
    let ramp: Vec<(f64, DynamicColor)> = expand_stops(
        &spread,
        conic_interpolation_tag(interpolation.color_space),
        conic_hue_direction(interpolation.hue_method),
        true,
    )
    .into_iter()
    .map(|(offset, color)| (f64::from(offset), color))
    .collect();
    average_color(&ramp)
}

/// Color channels from 0 to 1, multiplied by alpha.
fn premultiply(color: CssColor) -> [f64; 4] {
    let alpha = f64::from(color.a) / 255.0;
    [
        f64::from(color.r) / 255.0 * alpha,
        f64::from(color.g) / 255.0 * alpha,
        f64::from(color.b) / 255.0 * alpha,
        alpha,
    ]
}

/// 8-bit channels of a premultiplied color; a transparent one is black.
fn unpremultiply(channels: [f64; 4]) -> [u8; 4] {
    let alpha = channels[3];
    let byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    if alpha <= 0.0 {
        return [0, 0, 0, 0];
    }
    [
        byte(channels[0] / alpha),
        byte(channels[1] / alpha),
        byte(channels[2] / alpha),
        byte(alpha),
    ]
}

enum Normalized {
    Solid(Color),
    Stops {
        first: f64,
        last: f64,
        stops: Vec<(f32, DynamicColor)>,
    },
}

/// Map the stops onto the 0..1 range of the brush.
///
/// A non-repeating brush spans the gradient line or ray from 0% to 100%,
/// widened to reach stops outside it; pad extension shows the first and last
/// colors beyond the stops as CSS requires. A repeating brush spans one
/// period, from the first stop to the last. A repeating gradient whose stops
/// share one position paints the average color (CSS Images 3 §3.6); so does
/// one whose period, measured along `length` CSS pixels, is too short for any
/// output resolution to show.
fn normalize_stops(
    stops: Vec<(f64, DynamicColor)>,
    repeating: bool,
    length: f64,
    interpolation: &GradientColorInterpolation,
) -> Option<Normalized> {
    let (first, last) = (stops.first()?.0, stops.last()?.0);
    let period = if length > 0.0 {
        (last - first) * length
    } else {
        last - first
    };
    if repeating && period <= MIN_REPEATING_PERIOD {
        return Some(Normalized::Solid(ramp_average_color(&stops, interpolation)));
    }
    let (start, end) = if repeating {
        (first, last)
    } else {
        (first.min(0.0), last.max(1.0))
    };
    // Coincident stops make a hard transition. The brush needs strictly
    // increasing offsets to keep the later color past the shared position,
    // so the stops of one cluster spread over at most one negligible step,
    // however many there are.
    let step = (end - start) * f64::from(COINCIDENT_STEP);
    let mut stops = stops;
    let mut cluster_start = 0;
    while cluster_start < stops.len() {
        let position = stops[cluster_start].0;
        let mut cluster_end = cluster_start + 1;
        while cluster_end < stops.len() && stops[cluster_end].0 <= position {
            cluster_end += 1;
        }
        let count = cluster_end - cluster_start;
        if count > 1 {
            // Stay clear of the next stop so the order is kept.
            let width = stops
                .get(cluster_end)
                .map_or(step, |next| step.min((next.0 - position) / 2.0));
            let spacing = width / (count - 1) as f64;
            for (index, stop) in stops[cluster_start..cluster_end].iter_mut().enumerate() {
                stop.0 = position + spacing * index as f64;
            }
        }
        cluster_start = cluster_end;
    }
    let previous = stops.last().map_or(start, |stop| stop.0);
    let end = if repeating {
        previous
    } else {
        end.max(previous)
    };
    let span = end - start;
    let stops = stops
        .into_iter()
        .map(|(offset, color)| (((offset - start) / span) as f32, color))
        .collect();
    Some(Normalized::Stops {
        first: start,
        last: end,
        stops,
    })
}

fn finish_brush(
    brush: PenikoGradient,
    stops: &[(f32, DynamicColor)],
    repeating: bool,
    color_space: MixColorSpace,
    hue_method: HueInterpolationMethod,
) -> PenikoGradient {
    let space = conic_interpolation_tag(color_space);
    let brush = brush.with_extend(if repeating {
        PenikoExtend::Repeat
    } else {
        PenikoExtend::Pad
    });
    if !needs_expansion(stops, space) {
        return brush.with_stops(stops);
    }
    let stops = expand_stops(stops, space, conic_hue_direction(hue_method), repeating);
    brush.with_stops(stops.as_slice())
}

/// The fraction of the brush that separates coincident stops.
const COINCIDENT_STEP: f32 = 1e-5;

/// Shortest segment approximated with intermediate sRGB stops. Only the
/// steps between coincident stops are shorter, and those are hard
/// transitions with nothing to interpolate.
const MIN_EXPANDED_SEGMENT: f32 = 2.0 * COINCIDENT_STEP;

/// Exact samples across the whole brush, from offset 0 to 1, before each
/// gap is refined further.
const BRUSH_GRID_STEPS: usize = 64;

/// Largest premultiplied channel error, about half an 8-bit level, that a
/// drawn substep may have against the clipped ramp at its middle.
const CLIP_TOLERANCE: f32 = 0.5 / 255.0;

/// Stops a brush cannot take as they are: ones interpolated outside sRGB, or
/// ones outside its gamut, which a renderer clips before interpolating.
fn needs_expansion(stops: &[(f32, DynamicColor)], space: ColorSpaceTag) -> bool {
    space != ColorSpaceTag::Srgb || any_out_of_gamut(stops)
}

/// Some stop lies outside the sRGB gamut.
pub(super) fn any_out_of_gamut(stops: &[(f32, DynamicColor)]) -> bool {
    stops
        .iter()
        .any(|(_, color)| !in_gamut(color.to_alpha_color::<Srgb>()))
}

/// Every sRGB channel lies in 0..=1.
fn in_gamut(color: AlphaColor<Srgb>) -> bool {
    color.components[..3]
        .iter()
        .all(|channel| (0.0..=1.0).contains(channel))
}

/// The color clipped to the sRGB gamut, as it is drawn.
fn clamp_srgb(color: AlphaColor<Srgb>) -> AlphaColor<Srgb> {
    AlphaColor::<Srgb>::new(color.components.map(|channel| channel.clamp(0.0, 1.0)))
}

/// Push clipped samples strictly inside the part of the brush cell
/// `lo..hi` that lies in `segment`, at the cell's midpoints, halving it
/// until a renderer interpolating linearly between neighbors stays within
/// [`CLIP_TOLERANCE`] of the clipped ramp at each middle. A channel far
/// outside the gamut crosses into it within a sliver of a grid cell, which
/// the grid alone cannot find. Halving stops at [`MIN_EXPANDED_SEGMENT`]:
/// anything narrower draws as a hard transition, like the steps between
/// coincident stops. `at` samples the clipped ramp at a brush offset.
fn refine_cell(
    at: &impl Fn(f32) -> (f32, AlphaColor<Srgb>),
    segment: (f32, f32),
    (lo, hi): (f32, f32),
    out: &mut Vec<(f32, AlphaColor<Srgb>)>,
) {
    let (start, end) = (lo.max(segment.0), hi.min(segment.1));
    if end <= start || hi - lo < MIN_EXPANDED_SEGMENT {
        return;
    }
    let premultiplied = |color: AlphaColor<Srgb>| {
        let [r, g, b, a] = color.components;
        [r * a, g * a, b * a, a]
    };
    let (from, to, middle) = (
        premultiplied(at(start).1),
        premultiplied(at(end).1),
        premultiplied(at((start + end) / 2.0).1),
    );
    if (0..4).all(|channel| {
        ((from[channel] + to[channel]) / 2.0 - middle[channel]).abs() <= CLIP_TOLERANCE
    }) {
        return;
    }
    let mid = (lo + hi) / 2.0;
    refine_cell(at, segment, (lo, mid), out);
    if mid > segment.0 && mid < segment.1 {
        out.push(at(mid));
    }
    refine_cell(at, segment, (mid, hi), out);
}

/// Approximate interpolation in `space` with dense sRGB stops.
///
/// The colors outside the first and last stops are solid (CSS Images 3
/// §3.5.1), but a renderer padding a brush interpolated in a polar space
/// draws the pad as a segment between two equal colors, which `longer hue`
/// turns into a full hue turn. Expanding each segment here and pinning the
/// first and last colors to 0 and 1 keeps the pads solid.
pub(super) fn expand_stops(
    stops: &[(f32, DynamicColor)],
    space: ColorSpaceTag,
    direction: HueDirection,
    repeating: bool,
) -> Vec<(f32, DynamicColor)> {
    let mut expanded = Vec::with_capacity(stops.len() * 8);
    if let Some((offset, color)) = stops.first()
        && !repeating
        && *offset > 0.0
    {
        expanded.push((0.0, *color));
    }
    for pair in stops.windows(2) {
        let ((from_offset, from), (to_offset, to)) = (pair[0], pair[1]);
        let span = to_offset - from_offset;
        // Equal colors still turn a full circle with `longer hue`.
        if span < MIN_EXPANDED_SEGMENT || (from == to && direction != HueDirection::Longer) {
            expanded.push((from_offset, from));
            continue;
        }
        let exact = from.interpolate(to, space, direction);
        let sample = |t: f32| clamp_srgb(exact.eval(t).to_alpha_color::<Srgb>());
        let mut push_samples = |samples: Vec<(f32, AlphaColor<Srgb>)>| {
            for (t, color) in samples {
                expanded.push((
                    from_offset + span * t,
                    DynamicColor::from_alpha_color(color),
                ));
            }
        };
        // Sample the clipped ramp exactly at brush offsets that do not
        // depend on where the segment starts or ends: the points of a fixed
        // grid, refined by halving its cells. A segment split by a stop that
        // lies on the ramp is then sampled where the whole one is, except in
        // the one cell holding the split.
        let mut samples = vec![(0.0, sample(0.0))];
        let at = |offset: f32| {
            let t = ((offset - from_offset) / span).clamp(0.0, 1.0);
            (t, sample(t))
        };
        let first = (from_offset * BRUSH_GRID_STEPS as f32).floor() as i64;
        let last = (to_offset * BRUSH_GRID_STEPS as f32).ceil() as i64;
        for cell in first..last {
            let lo = cell as f32 / BRUSH_GRID_STEPS as f32;
            let hi = (cell + 1) as f32 / BRUSH_GRID_STEPS as f32;
            refine_cell(&at, (from_offset, to_offset), (lo, hi), &mut samples);
            if hi > from_offset && hi < to_offset {
                samples.push(at(hi));
            }
        }
        push_samples(samples);
    }
    if let Some((offset, color)) = stops.last() {
        expanded.push((*offset, *color));
        if !repeating && *offset < 1.0 {
            expanded.push((1.0, *color));
        }
    }
    expanded
}
