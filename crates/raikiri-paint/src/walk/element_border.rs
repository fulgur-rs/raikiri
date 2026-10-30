//! The square border of a box, painted as one strip per side (CSS Backgrounds
//! 3 §5).

use super::*;
use kurbo::{Circle, Shape};

/// The square border of one box. A `double` side draws its outer and inner
/// thirds and leaves the middle one transparent (§5.6); a `dashed` or `dotted`
/// side draws dashes or dots along its strip (§5.2); every other style that has
/// a color fills the whole strip.
pub(super) struct ElementBorder<'a> {
    border: &'a Sides<raikiri_style::resolve::ComputedBorder>,
    current_color: CssColor,
    /// The border box, its edges rounded to whole pixels.
    edges: Rect,
    /// The border widths in pixels.
    widths: Sides<f64>,
    /// Whether the top strip is painted: a continuation fragment has none.
    paint_top: bool,
}

impl<'a> ElementBorder<'a> {
    /// The border of a box `width` × `height` at (`abs_x`, `abs_y`). `None`
    /// when the box is empty or no side has a width.
    pub(super) fn new(
        width: f32,
        height: f32,
        abs_x: f32,
        abs_y: f32,
        border: &'a Sides<raikiri_style::resolve::ComputedBorder>,
        current_color: CssColor,
        paint_top: bool,
    ) -> Option<Self> {
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let widths = Sides {
            top: border.top.width().px() as f64,
            right: border.right.width().px() as f64,
            bottom: border.bottom.width().px() as f64,
            left: border.left.width().px() as f64,
        };
        let any_width =
            widths.left > 0.0 || widths.top > 0.0 || widths.right > 0.0 || widths.bottom > 0.0;
        any_width.then(|| Self {
            border,
            current_color,
            edges: Rect::new(
                (abs_x as f64).round(),
                (abs_y as f64).round(),
                ((abs_x + width) as f64).round(),
                ((abs_y + height) as f64).round(),
            ),
            widths,
            paint_top,
        })
    }

    /// Paints the strips of the sides: top, bottom, left and right.
    pub(super) fn paint(&self, scene: &mut impl PaintScene) {
        let Rect { x0, y0, x1, y1 } = self.edges;
        let Sides {
            top: bt,
            right: br,
            bottom: bb,
            left: bl,
        } = self.widths;
        let inner_y0 = if self.paint_top {
            (y0 + bt).round()
        } else {
            y0
        };
        let inner_y1 = (y1 - bb).round();
        // The left and right strips only need a vertical span. When the
        // content width collapses to zero (a border-only quadrant where left
        // plus right exactly fill the border box, as in the conic reference
        // files), the full inner box is empty but both side strips still
        // cover their rects.
        let inner_height_valid = inner_y1 > inner_y0;
        if self.paint_top && bt > 0.0 {
            let strip = Strip::horizontal(Rect::new(x0, y0, x1, y0 + bt));
            self.paint_side(scene, &self.border.top, strip);
        }
        if bb > 0.0 {
            let strip = Strip::horizontal(Rect::new(x0, y1 - bb, x1, y1));
            self.paint_side(scene, &self.border.bottom, strip);
        }
        if bl > 0.0 && inner_height_valid {
            let strip = Strip::vertical(Rect::new(x0, inner_y0, x0 + bl, inner_y1), (y0, y1));
            self.paint_side(scene, &self.border.left, strip);
        }
        if br > 0.0 && inner_height_valid {
            let strip = Strip::vertical(Rect::new(x1 - br, inner_y0, x1, inner_y1), (y0, y1));
            self.paint_side(scene, &self.border.right, strip);
        }
    }

    /// Paints one side in its style, unless its color is transparent.
    fn paint_side(
        &self,
        scene: &mut impl PaintScene,
        side: &raikiri_style::resolve::ComputedBorder,
        strip: Strip,
    ) {
        if let Some(color) = self.color(side) {
            fill_strip(scene, strip, color, side.style());
        }
    }

    /// The color a side is painted with, or `None` for a transparent one.
    /// `currentcolor` takes the text color. A side without a style has no
    /// width, so `paint` never asks for its color.
    fn color(&self, side: &raikiri_style::resolve::ComputedBorder) -> Option<Color> {
        let color = match side.color {
            raikiri_style::property::BorderColor::Resolved(color) => color,
            _ => self.current_color,
        };
        (color.a != 0).then(|| Color::from_rgba8(color.r, color.g, color.b, color.a))
    }
}

/// The strip of one side: the rectangle that is painted, and the extent of the
/// whole side along it, corners included.
#[derive(Clone, Copy)]
struct Strip {
    rect: Rect,
    /// Where the side starts and ends along the strip. A dashed or dotted
    /// pattern is laid out over it from end to end, and only the part inside
    /// `rect` is drawn: the corners belong to the top and bottom strips.
    line: (f64, f64),
    /// Whether the strip runs along the horizontal axis (the top and bottom
    /// sides) and not the vertical one.
    horizontal: bool,
}

impl Strip {
    /// The top or bottom strip, which spans the whole width of the box.
    fn horizontal(rect: Rect) -> Self {
        Self {
            rect,
            line: (rect.x0, rect.x1),
            horizontal: true,
        }
    }

    /// A left or right strip, between the top and bottom ones, of a side that
    /// is `line` long.
    fn vertical(rect: Rect, line: (f64, f64)) -> Self {
        Self {
            rect,
            line,
            horizontal: false,
        }
    }

    /// The extent of the strip across its axis, the border width of the side.
    fn across(&self) -> (f64, f64) {
        if self.horizontal {
            (self.rect.y0, self.rect.y1)
        } else {
            (self.rect.x0, self.rect.x1)
        }
    }

    /// The extent of the strip along its axis.
    fn visible(&self) -> (f64, f64) {
        if self.horizontal {
            (self.rect.x0, self.rect.x1)
        } else {
            (self.rect.y0, self.rect.y1)
        }
    }
}

/// Fills the `strip` of one side in its `style`.
///
/// `double` draws the outer and inner thirds of the thickness (§5.6, "two
/// parallel solid lines"). Below 3px the thirds vanish, so a small double falls
/// back to solid, which matches the clamping of browsers closely enough for
/// reftests. A `dashed` or `dotted` side that is too short or too long for a
/// pattern is filled solid.
fn fill_strip(scene: &mut impl PaintScene, strip: Strip, color: Color, style: BorderStyle) {
    let Rect { x0, y0, x1, y1 } = strip.rect;
    let (across0, across1) = strip.across();
    let thickness = across1 - across0;
    let dotted = style == BorderStyle::Dotted;
    let dashes = if dotted || style == BorderStyle::Dashed {
        dash_lengths(strip.line.1 - strip.line.0, thickness, dotted)
    } else {
        None
    };
    if let Some(dashes) = dashes {
        fill_dashes(scene, strip, color, dotted, dashes);
    } else if style == BorderStyle::Double && thickness >= 3.0 {
        let third = thickness / 3.0;
        let (outer, inner) = if strip.horizontal {
            (
                Rect::new(x0, y0, x1, y0 + third),
                Rect::new(x0, y1 - third, x1, y1),
            )
        } else {
            (
                Rect::new(x0, y0, x0 + third, y1),
                Rect::new(x1 - third, y0, x1, y1),
            )
        };
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &outer);
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &inner);
    } else {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &strip.rect);
    }
}

/// The tolerance with which the circle of a round dot is flattened to curves,
/// the one the other paths of the painter use.
const DOT_TOLERANCE: f64 = 0.1;

/// Draws the `dashes` of a dashed or dotted side as one path, so that a long
/// side costs the scene one fill rather than one per dash.
///
/// Dashes are square-ended, and so are the dots of a border up to 3px wide;
/// wider dots are round. The pattern restarts at the start of each fragment of
/// a box that breaks across pages, as the sides of every fragment are laid out
/// on their own.
fn fill_dashes(
    scene: &mut impl PaintScene,
    strip: Strip,
    color: Color,
    dotted: bool,
    dashes: Dashes,
) {
    let Dashes { dash, gap, count } = dashes;
    let (start, end) = strip.line;
    let (across0, across1) = strip.across();
    let (visible0, visible1) = strip.visible();
    let thickness = across1 - across0;
    let period = dash + gap;
    let round = dotted && thickness > 3.0;
    let mut path = BezPath::new();
    for index in 0..count {
        let from = start + index as f64 * period;
        if round {
            let center = from + dash / 2.0;
            // A dot in a corner belongs to the strip that owns the corner.
            if center + thickness / 2.0 <= visible0 || center - thickness / 2.0 >= visible1 {
                continue;
            }
            let across = (across0 + across1) / 2.0;
            let at = if strip.horizontal {
                (center, across)
            } else {
                (across, center)
            };
            path.extend(Circle::new(at, thickness / 2.0).path_elements(DOT_TOLERANCE));
        } else {
            let (from, to) = (from.max(visible0), (from + dash).min(visible1));
            if from >= to {
                continue;
            }
            let piece = if strip.horizontal {
                Rect::new(from, across0, to, across1)
            } else {
                Rect::new(across0, from, across1, to)
            };
            path.extend(piece.path_elements(0.0));
        }
    }
    if path.is_empty() {
        return;
    }
    // A round dot at the end of the visible part sticks out of the strip.
    let clip = round && (visible0 > start || visible1 < end);
    if clip {
        scene.push_clip_layer(Affine::IDENTITY, &strip.rect);
    }
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &path);
    if clip {
        scene.pop_layer();
    }
}

/// The most dashes or dots one side draws. A longer side is filled solid, which
/// bounds the size of the path of a box with a hostile but finite size.
const MAX_SIDE_DASHES: f64 = 4096.0;

/// The dashes of a side: their length, the gap between two, and their count.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dashes {
    dash: f64,
    gap: f64,
    count: usize,
}

/// The dashes of a dashed or dotted side `length` long and `thickness` thick,
/// or `None` when the side cannot hold two of them, holds more than
/// [`MAX_SIDE_DASHES`], or has no finite size, and is filled solid.
///
/// A dash is twice the thickness and a gap once from 3px on, three and two times
/// below it; a dot is as long as it is thick and a gap the same. The gap is then
/// stretched or shrunk so that the pattern starts and ends with a dash, whichever
/// of the two nearest dash counts leaves it closest to its natural length. These
/// are the lengths Chromium draws, which matter when a page is compared with
/// its render.
fn dash_lengths(length: f64, thickness: f64, dotted: bool) -> Option<Dashes> {
    let (dash, gap) = match (dotted, thickness >= 3.0) {
        (true, _) => (thickness, thickness),
        (false, true) => (2.0 * thickness, thickness),
        (false, false) => (3.0 * thickness, 2.0 * thickness),
    };
    if !length.is_finite() || !thickness.is_finite() || length <= 2.0 * dash {
        return None;
    }
    if length <= 2.0 * dash + gap {
        // Exactly two dashes, in proportion.
        let scale = length / (2.0 * dash + gap);
        return Some(Dashes {
            dash: dash * scale,
            gap: gap * scale,
            count: 2,
        });
    }
    let fewer = ((length + 2.0 * gap) / (dash + gap)).floor();
    let more = fewer + 1.0;
    if more > MAX_SIDE_DASHES {
        return None;
    }
    let gap_for = |dashes: f64| (length - dashes * dash) / (dashes - 1.0);
    let (fewer_gap, more_gap) = (gap_for(fewer), gap_for(more));
    let (count, gap) = if more_gap <= 0.0 || (fewer_gap - gap).abs() < (more_gap - gap).abs() {
        (fewer, fewer_gap)
    } else {
        (more, more_gap)
    };
    Some(Dashes {
        dash,
        gap,
        count: count as usize,
    })
}

#[cfg(test)]
mod tests;
