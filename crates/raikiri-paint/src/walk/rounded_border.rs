//! The border of a box with rounded corners whose sides are not all painted
//! alike, or whose style is drawn in bands (CSS Backgrounds 3 §4, §5).
//!
//! Every band of the border follows the curves: a band is the ring between
//! two rounded rectangles, the border box inset by where the band starts and
//! by where it ends on each side, their radii shrinking by the same amounts
//! (§4.2). A `double` side is two such bands, a `groove` or `ridge` side two
//! of different shades, and an `inset` or `outset` side one band whose shade
//! depends on the side.
//!
//! Where two sides meet at a corner and are painted differently, the corner
//! is cut along the line from the outer corner of the box toward the inner
//! one, as Chromium cuts it, and each side paints its part of the curved band
//! (§5.4). Sides that are painted alike are painted together, so no seam
//! shows between them.

use super::element_border::{band_color, bands};
use super::*;

/// The sides of a border in the order of the corners that follow them:
/// top, right, bottom and left.
const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// Paints the border `border` of the box at `rect` with the used corner
/// `radii`, every band following the curves. Returns `false`, having painted
/// nothing, when a side with a width is `dashed` or `dotted`, which this
/// painter does not draw.
pub(super) fn paint_rounded_border_bands(
    scene: &mut impl PaintScene,
    (abs_x, abs_y, width, height): (f32, f32, f32, f32),
    radii: [[f64; 2]; 4],
    border: &Sides<raikiri_style::resolve::ComputedBorder>,
    current_color: CssColor,
) -> bool {
    let sides = [&border.top, &border.right, &border.bottom, &border.left];
    let widths = sides.map(|side| f64::from(side.width().px()));
    if sides.iter().zip(widths).any(|(side, width)| {
        width > 0.0 && matches!(side.style(), BorderStyle::Dashed | BorderStyle::Dotted)
    }) {
        return false;
    }
    let (x0, y0) = (f64::from(abs_x), f64::from(abs_y));
    let (x1, y1) = (f64::from(abs_x + width), f64::from(abs_y + height));
    if x1 <= x0 || y1 <= y0 || widths.iter().all(|width| *width <= 0.0) {
        return true;
    }
    let clips = side_clips(Rect::new(x0, y0, x1, y1), radii, widths);
    for ring in 0..2 {
        let bands = std::array::from_fn::<_, 4, _>(|index| {
            bands(
                sides[index].style(),
                widths[index],
                matches!(index, TOP | LEFT),
            )[ring]
        });
        if bands.iter().all(|band| band.width <= 0.0) {
            continue;
        }
        let inset = |depth: fn(&super::element_border::Band) -> f64| {
            let [top, right, bottom, left] = bands.map(|band| depth(&band));
            rounded_rect_path(
                x0 + left,
                y0 + top,
                x1 - right,
                y1 - bottom,
                inset_border_radii(radii, (left, top, right, bottom)),
            )
        };
        let mut band_path = inset(|band| band.offset);
        band_path.extend(
            inset(|band| band.offset + band.width)
                .reverse_subpaths()
                .iter(),
        );
        // The sides with a band in this ring, grouped by the color they paint
        // it with; a transparent side paints nothing.
        let mut groups: Vec<(Color, Vec<usize>)> = Vec::new();
        for index in [TOP, RIGHT, BOTTOM, LEFT] {
            if bands[index].width <= 0.0 {
                continue;
            }
            let Some(color) = band_color(
                sides[index],
                ring,
                matches!(index, TOP | LEFT),
                current_color,
            ) else {
                continue;
            };
            let color = Color::from_rgba8(color.r, color.g, color.b, color.a);
            match groups.iter_mut().find(|(group, _)| *group == color) {
                Some((_, members)) => members.push(index),
                None => groups.push((color, vec![index])),
            }
        }
        let whole = groups.len() == 1
            && (0..4).all(|index| bands[index].width <= 0.0 || groups[0].1.contains(&index));
        for (color, members) in groups {
            if whole {
                scene.fill(Fill::EvenOdd, Affine::IDENTITY, color, None, &band_path);
                continue;
            }
            let mut clip = BezPath::new();
            for index in members {
                clip.extend(clips[index].iter());
            }
            scene.push_clip_layer(Affine::IDENTITY, &clip);
            scene.fill(Fill::EvenOdd, Affine::IDENTITY, color, None, &band_path);
            scene.pop_layer();
        }
    }
    true
}

/// The part of the box `edges` that belongs to each side, top, right, bottom
/// and left, as closed paths that wind the same way, so that the parts of
/// several sides fill as one region without a seam between them.
///
/// Each corner is cut in two along the line from the outer corner of the box
/// in the direction of the inner corner of the border, which the border
/// widths of its two sides set. The cut runs across the box of the corner:
/// the larger of its radius and the width of the side, on each axis, which
/// holds all of the curved border there.
fn side_clips(edges: Rect, radii: [[f64; 2]; 4], widths: [f64; 4]) -> [BezPath; 4] {
    let Rect { x0, y0, x1, y1 } = edges;
    let (w, h) = (edges.width(), edges.height());
    let [top, right, bottom, left] = widths;
    // The box of each corner, as its size along x and y, top-left first and
    // clockwise.
    let mut boxes = [
        [radii[0][0].max(left), radii[0][1].max(top)],
        [radii[1][0].max(right), radii[1][1].max(top)],
        [radii[2][0].max(right), radii[2][1].max(bottom)],
        [radii[3][0].max(left), radii[3][1].max(bottom)],
    ];
    // Two corners on one side of the box may not overlap: where the boxes
    // of the corners at the ends of a side would, they share the side in
    // proportion to their sizes.
    let share = |a: &mut f64, b: &mut f64, length: f64| {
        let sum = *a + *b;
        if sum > length {
            *a *= length / sum;
            *b *= length / sum;
        }
    };
    let [[tl_w, tl_h], [tr_w, tr_h], [br_w, br_h], [bl_w, bl_h]] = &mut boxes;
    share(tl_w, tr_w, w);
    share(bl_w, br_w, w);
    share(tl_h, bl_h, h);
    share(tr_h, br_h, h);
    // Where the cut of a corner leaves its box, measured from the outer
    // corner along x and y (both positive), for a cut toward (`wx`, `wy`).
    let exit = |[bx, by]: [f64; 2], wx: f64, wy: f64| -> [f64; 2] {
        if wx <= 0.0 && wy <= 0.0 {
            return [bx, by];
        }
        let t = match (wx > 0.0, wy > 0.0) {
            (true, true) => (bx / wx).min(by / wy),
            (true, false) => bx / wx,
            _ => by / wy,
        };
        [wx * t, wy * t]
    };
    let tl = boxes[0];
    let tr = boxes[1];
    let br = boxes[2];
    let bl = boxes[3];
    let [tl_x, tl_y] = exit(tl, left, top);
    let [tr_x, tr_y] = exit(tr, right, top);
    let [br_x, br_y] = exit(br, right, bottom);
    let [bl_x, bl_y] = exit(bl, left, bottom);
    let polygon = |points: &[Point]| {
        let mut path = BezPath::new();
        path.move_to(points[0]);
        for point in &points[1..] {
            path.line_to(*point);
        }
        path.close_path();
        path
    };
    // Each part runs clockwise: along the outer edge of its side, then back
    // along the inner side of the corner boxes, through the cut ends.
    let top_part = polygon(&[
        Point::new(x0, y0),
        Point::new(x1, y0),
        Point::new(x1 - tr_x, y0 + tr_y),
        Point::new(x1 - tr[0], y0 + tr[1]),
        Point::new(x1 - tr[0], y0 + top.max(tr[1])),
        Point::new(x0 + tl[0], y0 + top.max(tl[1])),
        Point::new(x0 + tl[0], y0 + tl[1]),
        Point::new(x0 + tl_x, y0 + tl_y),
    ]);
    let right_part = polygon(&[
        Point::new(x1, y0),
        Point::new(x1, y1),
        Point::new(x1 - br_x, y1 - br_y),
        Point::new(x1 - br[0], y1 - br[1]),
        Point::new(x1 - right.max(br[0]), y1 - br[1]),
        Point::new(x1 - right.max(tr[0]), y0 + tr[1]),
        Point::new(x1 - tr[0], y0 + tr[1]),
        Point::new(x1 - tr_x, y0 + tr_y),
    ]);
    let bottom_part = polygon(&[
        Point::new(x1, y1),
        Point::new(x0, y1),
        Point::new(x0 + bl_x, y1 - bl_y),
        Point::new(x0 + bl[0], y1 - bl[1]),
        Point::new(x0 + bl[0], y1 - bottom.max(bl[1])),
        Point::new(x1 - br[0], y1 - bottom.max(br[1])),
        Point::new(x1 - br[0], y1 - br[1]),
        Point::new(x1 - br_x, y1 - br_y),
    ]);
    let left_part = polygon(&[
        Point::new(x0, y1),
        Point::new(x0, y0),
        Point::new(x0 + tl_x, y0 + tl_y),
        Point::new(x0 + tl[0], y0 + tl[1]),
        Point::new(x0 + left.max(tl[0]), y0 + tl[1]),
        Point::new(x0 + left.max(bl[0]), y1 - bl[1]),
        Point::new(x0 + bl[0], y1 - bl[1]),
        Point::new(x0 + bl_x, y1 - bl_y),
    ]);
    [top_part, right_part, bottom_part, left_part]
}
