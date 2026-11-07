//! The dashed or dotted border of a box with rounded corners (CSS Backgrounds 3
//! §5.2), laid out along the curve.
//!
//! Chromium strokes the centerline of the border, a rounded rectangle half a
//! border width inside the outer edge, as one closed path: the pattern starts on
//! the top edge where the upper-left corner ends and runs clockwise, and the gap
//! is chosen for the length of the whole path, so that the pattern fits it. The
//! pattern is clipped to the border, which an elliptical corner would not
//! contain. It follows the curve only when the inner edge of the border is
//! rounded too: a border no thicker than its radii is drawn in straight sides,
//! each from its own corner, inside the rounded outer edge.

use super::element_border::{Dashes, closed_dash_lengths};
use super::*;
use kurbo::{Cap, Circle, ParamCurve, ParamCurveArclen, PathEl, Shape, Stroke};

/// The accuracy of the arc lengths, in px.
const ACCURACY: f64 = 1.0e-3;

/// The tolerance with which the circle of a round dot is flattened to curves.
const DOT_TOLERANCE: f64 = 0.1;

/// How much wider than the border the stroke of the dashes is, in px: a pixel
/// on each side, so that the edges of the dashes are antialiased by the clip
/// alone.
const STROKE_BLEED: f64 = 2.0;

/// Paint the dashed or dotted border of `thickness` that is inside the rounded
/// outer edge of the box at (`abs_x`, `abs_y`), where all four sides share the
/// width and the color. A border that cannot hold two dashes is painted solid,
/// as Chromium does.
pub(super) fn paint_rounded_dashed_border(
    scene: &mut impl PaintScene,
    (abs_x, abs_y, width, height): (f32, f32, f32, f32),
    radii: [[f64; 2]; 4],
    thickness: f64,
    color: Color,
    dotted: bool,
) {
    let edges = snapped_edges((abs_x, abs_y, width, height));
    let half = thickness / 2.0;
    let centerline = rounded_rect_path(
        edges.x0 + half,
        edges.y0 + half,
        edges.x1 - half,
        edges.y1 - half,
        inset_border_radii(radii, (half, half, half, half)),
    );
    let Some(dashes) = closed_dash_lengths(centerline.perimeter(ACCURACY), thickness, dotted)
    else {
        fill_rounded_border_ring(
            scene,
            (abs_x, abs_y, width, height),
            radii,
            thickness,
            color,
        );
        return;
    };
    // The pattern starts where the upper-left corner ends.
    let centerline = from_end_of_first_curve(&centerline);
    // The pattern is clipped to the border, as Chromium does: next to an
    // elliptical corner, or one that is rounder than the border is wide, the
    // stroke of the centerline reaches past the ring that the border covers.
    scene.push_clip_layer(Affine::IDENTITY, &border_ring(edges, radii, thickness));
    if dotted && thickness > 3.0 {
        paint_dots(scene, &centerline, dashes, thickness, color);
    } else {
        let dashed: BezPath =
            kurbo::dash(centerline.iter(), 0.0, &[dashes.dash, dashes.gap]).collect();
        let stroke = Stroke::new(thickness + STROKE_BLEED).with_caps(Cap::Butt);
        scene.stroke(&stroke, Affine::IDENTITY, color, None, &dashed);
    }
    scene.pop_layer();
}

/// The box at (`abs_x`, `abs_y`) snapped to whole pixels, as the border of a
/// square box is.
fn snapped_edges((abs_x, abs_y, width, height): (f32, f32, f32, f32)) -> Rect {
    Rect::new(
        f64::from(abs_x).round(),
        f64::from(abs_y).round(),
        f64::from(abs_x + width).round(),
        f64::from(abs_y + height).round(),
    )
}

/// Whether the inner edge of a border `thickness` wide is rounded at any
/// corner. Chromium follows the curve with a dashed or dotted border only
/// then; when the radii are no larger than the border is wide, it draws the
/// sides straight, each from its own corner, inside the rounded outer edge.
pub(super) fn inner_edge_is_rounded(radii: [[f64; 2]; 4], thickness: f64) -> bool {
    inset_border_radii(radii, (thickness, thickness, thickness, thickness))
        .iter()
        .any(|radius| *radius != [0.0; 2])
}

/// Run `paint` with `scene` clipped to the rounded outer edge of the box at
/// (`abs_x`, `abs_y`).
pub(super) fn within_rounded_outer_edge<S: PaintScene>(
    scene: &mut S,
    rect: (f32, f32, f32, f32),
    radii: [[f64; 2]; 4],
    paint: impl FnOnce(&mut S),
) {
    let edges = snapped_edges(rect);
    let outer = rounded_rect_path(edges.x0, edges.y0, edges.x1, edges.y1, radii);
    scene.push_clip_layer(Affine::IDENTITY, &outer);
    paint(scene);
    scene.pop_layer();
}

/// The ring between the rounded outer edge of the box `edges` and the inner
/// edge of a border `thickness` wide, as a path that a clip fills by its
/// winding.
fn border_ring(edges: Rect, radii: [[f64; 2]; 4], thickness: f64) -> BezPath {
    let mut ring = rounded_rect_path(edges.x0, edges.y0, edges.x1, edges.y1, radii);
    // A box that is not wider and taller than twice the border has no hole.
    if edges.width() > 2.0 * thickness && edges.height() > 2.0 * thickness {
        let inner = rounded_rect_path(
            edges.x0 + thickness,
            edges.y0 + thickness,
            edges.x1 - thickness,
            edges.y1 - thickness,
            inset_border_radii(radii, (thickness, thickness, thickness, thickness)),
        );
        ring.extend(inner.reverse_subpaths().iter());
    }
    ring
}

/// `path` with the curve at its start moved to its end, so that it begins where
/// that curve ends: a rounded rectangle then starts on its top edge, where the
/// upper-left corner ends, and still runs clockwise. A path that starts with a
/// straight line is returned as it is.
fn from_end_of_first_curve(path: &BezPath) -> BezPath {
    let elements = path.elements();
    let tail = elements.get(1..).unwrap_or_default();
    let curves = tail
        .iter()
        .take_while(|element| !matches!(element, PathEl::LineTo(_) | PathEl::ClosePath))
        .count();
    let (leading, rest) = tail.split_at(curves);
    // Without a curve before the first line, the path already starts where it
    // should.
    let (Some(PathEl::MoveTo(origin)), Some(end)) = (
        elements.first().copied(),
        leading.last().and_then(|element| element.end_point()),
    ) else {
        return path.clone();
    };
    let mut rotated = BezPath::new();
    rotated.move_to(end);
    // The closing line of the path is a line like the others now.
    rotated.extend(
        rest.iter()
            .copied()
            .filter(|e| !matches!(e, PathEl::ClosePath)),
    );
    rotated.line_to(origin);
    rotated.extend(leading.iter().copied());
    rotated.close_path();
    rotated
}

/// Paint the round dots of a dotted border wider than 3px, one every dash and
/// gap along `path` from its start, as one fill.
fn paint_dots(
    scene: &mut impl PaintScene,
    path: &BezPath,
    dashes: Dashes,
    thickness: f64,
    color: Color,
) {
    let period = dashes.dash + dashes.gap;
    // The segments of the path with the length at which each begins.
    let mut segments = Vec::new();
    let mut length = 0.0;
    for segment in path.segments() {
        segments.push((length, segment));
        length += segment.arclen(ACCURACY);
    }
    let mut dots = BezPath::new();
    for index in 0..dashes.count {
        let at = index as f64 * period;
        let position = segments.partition_point(|(begin, _)| *begin <= at);
        let (begin, segment) = segments[position.saturating_sub(1)];
        let t = segment.inv_arclen(at - begin, ACCURACY);
        let center = segment.eval(t);
        dots.extend(Circle::new(center, thickness / 2.0).path_elements(DOT_TOLERANCE));
    }
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &dots);
}
