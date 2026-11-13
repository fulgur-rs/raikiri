//! The square border of a box, painted as one strip per side (CSS Backgrounds
//! 3 §5).

use super::*;
use crate::bevel;
use kurbo::{Circle, Shape};

/// The square border of one box. A `double` side draws its outer and inner
/// thirds and leaves the middle one transparent (§5.6); a `dashed` or `dotted`
/// side draws dashes or dots along its strip (§5.2); a `groove` or `ridge` side
/// is two bands of a lighter and a darker shade, and an `inset` or `outset`
/// one a single band of either (§3.2); every other style that has a color fills
/// the whole strip.
///
/// Where the two sides of a corner are painted in the same color, the top or
/// bottom strip runs over the corner. Where they are not, because the colors or
/// the shades of a bevel differ or one side is transparent, the corner is cut in
/// two along the diagonal from the outer to the inner corner and each side keeps
/// its half (§5.4, and the way Chromium paints it).
pub(super) struct ElementBorder<'a> {
    border: &'a Sides<raikiri_style::resolve::ComputedBorder>,
    current_color: CssColor,
    /// The border box, its edges rounded to whole pixels.
    edges: Rect,
    /// The border widths in pixels.
    widths: Sides<f64>,
    /// Whether the top strip is painted: a continuation fragment has none.
    paint_top: bool,
    /// Which corners are cut along their diagonal.
    miters: Miters,
    /// For a cell of a table with collapsed borders, which side paints each
    /// corner whole: such corners are never cut.
    corners: Option<raikiri_dom::paint_rules::CollapsedCorners>,
}

/// The corners of a box that are cut along their diagonal between the strips of
/// their two sides, each one a flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Miters {
    top_left: bool,
    top_right: bool,
    bottom_right: bool,
    bottom_left: bool,
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
            miters: corner_miters(border, current_color, paint_top),
            corners: None,
        })
    }

    /// The same border, with each corner painted whole by the side `corners`
    /// names, as a cell of a table with collapsed borders paints it.
    pub(super) fn with_corners(
        mut self,
        corners: raikiri_dom::paint_rules::CollapsedCorners,
    ) -> Self {
        self.miters = Miters::default();
        self.corners = Some(corners);
        self
    }

    /// Paints the strips of the sides: top, bottom, left and right. A side that
    /// has a second, inner band (`groove`, `ridge` and `double`) has it painted
    /// as a second ring of strips inside the first.
    pub(super) fn paint(&self, scene: &mut impl PaintScene) {
        let outer = self.ring(0);
        self.paint_ring(scene, 0, self.edges, outer.widths);
        let inner = self.ring(1);
        let widths = inner.widths;
        if widths.top > 0.0 || widths.right > 0.0 || widths.bottom > 0.0 || widths.left > 0.0 {
            let Rect { x0, y0, x1, y1 } = self.edges;
            let offsets = inner.offsets;
            let top = if self.paint_top { y0 + offsets.top } else { y0 };
            let edges = Rect::new(
                x0 + offsets.left,
                top,
                x1 - offsets.right,
                y1 - offsets.bottom,
            );
            // Bands wider than the box have no room for an inner ring.
            if edges.x1 >= edges.x0 && edges.y1 >= edges.y0 {
                self.paint_ring(scene, 1, edges, widths);
            }
        }
    }

    /// The band of each side that is in ring `ring`, the outer ring being 0.
    fn ring(&self, ring: usize) -> Ring {
        let band = |side: &raikiri_style::resolve::ComputedBorder, width: f64, top_left: bool| {
            bands(side.style(), width, top_left)[ring]
        };
        let (top, right, bottom, left) = (
            band(&self.border.top, self.widths.top, true),
            band(&self.border.right, self.widths.right, false),
            band(&self.border.bottom, self.widths.bottom, false),
            band(&self.border.left, self.widths.left, true),
        );
        Ring {
            offsets: Sides {
                top: top.offset,
                right: right.offset,
                bottom: bottom.offset,
                left: left.offset,
            },
            widths: Sides {
                top: top.width,
                right: right.width,
                bottom: bottom.width,
                left: left.width,
            },
        }
    }

    /// Paints the strips of ring `ring` of the sides, `widths` thick, around
    /// `edges`.
    fn paint_ring(
        &self,
        scene: &mut impl PaintScene,
        ring: usize,
        edges: Rect,
        widths: Sides<f64>,
    ) {
        let Rect { x0, y0, x1, y1 } = edges;
        let Sides {
            top: bt,
            right: br,
            bottom: bb,
            left: bl,
        } = widths;
        let inner_y0 = if self.paint_top {
            (y0 + bt).round()
        } else {
            y0
        };
        let inner_y1 = (y1 - bb).round();
        // Where a corner is cut along its diagonal, the strip of each of its
        // sides runs over the corner, to the edge of the box, and ends in the
        // cut.
        let Miters {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        } = self.miters;
        // The diagonal of a corner runs from the outer corner of the box to the
        // inner one, whichever ring a strip belongs to.
        let Rect {
            x0: box_x0,
            y0: box_y0,
            x1: box_x1,
            y1: box_y1,
        } = self.edges;
        let Sides {
            top: wt,
            right: wr,
            bottom: wb,
            left: wl,
        } = self.widths;
        let cut = |mitered: bool, corner: (f64, f64), toward: (f64, f64)| {
            if mitered {
                Join::Miter {
                    corner: Point::new(corner.0, corner.1),
                    toward: Vec2::new(toward.0, toward.1),
                }
            } else {
                Join::Square
            }
        };
        let top_left_cut = cut(top_left, (box_x0, box_y0), (wl, wt));
        let top_right_cut = cut(top_right, (box_x1, box_y0), (-wr, wt));
        let bottom_right_cut = cut(bottom_right, (box_x1, box_y1), (-wr, -wb));
        let bottom_left_cut = cut(bottom_left, (box_x0, box_y1), (wl, -wb));
        // Which side runs over each corner of a collapsed-border cell. A
        // corner the horizontal side does not own keeps the horizontal strip
        // out of the corner square of the box; one the vertical side owns
        // runs the vertical strip over it.
        use raikiri_dom::paint_rules::CornerOwner;
        let owner = |pick: fn(&raikiri_dom::paint_rules::CollapsedCorners) -> CornerOwner| {
            self.corners.as_ref().map_or(CornerOwner::Horizontal, pick)
        };
        let (owner_tl, owner_tr, owner_br, owner_bl) = (
            owner(|c| c.top_left),
            owner(|c| c.top_right),
            owner(|c| c.bottom_right),
            owner(|c| c.bottom_left),
        );
        // Where a horizontal strip stops short of a corner it does not own:
        // at `corner`, or nowhere (`unbounded`) when it owns it.
        let stop = |owner: CornerOwner, corner: f64, unbounded: f64| {
            if matches!(owner, CornerOwner::Horizontal) {
                unbounded
            } else {
                corner
            }
        };
        // How far the junction square of each corner reaches past the
        // cell's own top or bottom line: half of what the widest horizontal
        // line there is wider.
        let heights = self
            .corners
            .map_or([0.0; 4], |corners| corners.junction_heights);
        // A cell without a line of its own there has no line center to
        // reach from, and leaves the square to its neighbours.
        let reach = |corner: usize, own: f64| {
            if own > 0.0 {
                ((f64::from(heights[corner]) - own) / 2.0).max(0.0)
            } else {
                0.0
            }
        };
        let (reach_tl, reach_tr, reach_br, reach_bl) =
            (reach(0, wt), reach(1, wt), reach(2, wb), reach(3, wb));
        if self.paint_top && bt > 0.0 {
            let strip = Strip::horizontal(
                Rect::new(
                    if top_left { box_x0 } else { x0 },
                    y0,
                    if top_right { box_x1 } else { x1 },
                    y0 + bt,
                ),
                top_left_cut,
                top_right_cut,
            )
            .within(
                stop(owner_tl, box_x0 + wl, f64::NEG_INFINITY),
                stop(owner_tr, box_x1 - wr, f64::INFINITY),
            );
            self.paint_side(scene, &self.border.top, strip, ring, true);
        }
        if bb > 0.0 {
            let strip = Strip::horizontal(
                Rect::new(
                    if bottom_left { box_x0 } else { x0 },
                    y1 - bb,
                    if bottom_right { box_x1 } else { x1 },
                    y1,
                ),
                bottom_left_cut,
                bottom_right_cut,
            )
            .within(
                stop(owner_bl, box_x0 + wl, f64::NEG_INFINITY),
                stop(owner_br, box_x1 - wr, f64::INFINITY),
            );
            self.paint_side(scene, &self.border.bottom, strip, ring, false);
        }
        // The left and right strips only need a vertical span. When the
        // content width collapses to zero (a border-only quadrant where left
        // plus right exactly fill the border box, as in the conic reference
        // files), the full inner box is empty but both side strips still
        // cover their rects.
        // A vertical strip runs to the box edge over a cut corner, to the
        // ring's edge over a corner it owns, and otherwise stops at the inner
        // edge of the horizontal side, or past the junction square.
        let span = |cut: bool, owner: CornerOwner, edge: (f64, f64, f64)| {
            let (box_edge, ring_edge, inner) = edge;
            if cut {
                box_edge
            } else if matches!(owner, CornerOwner::Vertical) {
                ring_edge
            } else {
                inner
            }
        };
        let left_span = (
            span(top_left, owner_tl, (box_y0, y0, inner_y0 + reach_tl)),
            span(bottom_left, owner_bl, (box_y1, y1, inner_y1 - reach_bl)),
        );
        if bl > 0.0 && left_span.1 > left_span.0 {
            let strip = Strip::vertical(
                Rect::new(x0, left_span.0, x0 + bl, left_span.1),
                (y0, y1),
                top_left_cut,
                bottom_left_cut,
            );
            self.paint_side(scene, &self.border.left, strip, ring, true);
        }
        let right_span = (
            span(top_right, owner_tr, (box_y0, y0, inner_y0 + reach_tr)),
            span(bottom_right, owner_br, (box_y1, y1, inner_y1 - reach_br)),
        );
        if br > 0.0 && right_span.1 > right_span.0 {
            let strip = Strip::vertical(
                Rect::new(x1 - br, right_span.0, x1, right_span.1),
                (y0, y1),
                top_right_cut,
                bottom_right_cut,
            );
            self.paint_side(scene, &self.border.right, strip, ring, false);
        }
        // A solid line of a neighbouring cell that takes a corner square is
        // painted there again, so that a cell painted after that neighbour,
        // as a positioned one is, does not cover the junction with its
        // background.
        let squares = [
            (
                owner_tl,
                Rect::new(
                    box_x0,
                    box_y0 - reach_tl,
                    box_x0 + wl,
                    box_y0 + wt + reach_tl,
                ),
            ),
            (
                owner_tr,
                Rect::new(
                    box_x1 - wr,
                    box_y0 - reach_tr,
                    box_x1,
                    box_y0 + wt + reach_tr,
                ),
            ),
            (
                owner_br,
                Rect::new(
                    box_x1 - wr,
                    box_y1 - wb - reach_br,
                    box_x1,
                    box_y1 + reach_br,
                ),
            ),
            (
                owner_bl,
                Rect::new(
                    box_x0,
                    box_y1 - wb - reach_bl,
                    box_x0 + wl,
                    box_y1 + reach_bl,
                ),
            ),
        ];
        for (owner, square) in squares {
            if let CornerOwner::Neither(line) = owner
                && ring == 0
                && line.style() == BorderStyle::Solid
                && square.area() > 0.0
                && let Some(color) = self.color(&line, 0, true)
            {
                scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &square);
            }
        }
    }

    /// Paints one side in its style, unless its color is transparent.
    /// `top_left` tells the top and left sides, which face the light, from the
    /// bottom and right ones.
    fn paint_side(
        &self,
        scene: &mut impl PaintScene,
        side: &raikiri_style::resolve::ComputedBorder,
        strip: Strip,
        ring: usize,
        top_left: bool,
    ) {
        if let Some(color) = self.color(side, ring, top_left) {
            fill_strip(scene, strip, color, side.style());
        }
    }

    /// The color a side is painted with in ring `ring`, or `None` for a
    /// transparent one. A side without a style has no width, so `paint` never
    /// asks for its color.
    fn color(
        &self,
        side: &raikiri_style::resolve::ComputedBorder,
        ring: usize,
        top_left: bool,
    ) -> Option<Color> {
        let color = band_color(side, ring, top_left, self.current_color)?;
        Some(Color::from_rgba8(color.r, color.g, color.b, color.a))
    }
}

/// The corners whose two sides are not drawn alike. A continuation fragment has
/// no top corners, and a side without a width has no corner to cut.
fn corner_miters(
    border: &Sides<raikiri_style::resolve::ComputedBorder>,
    current_color: CssColor,
    paint_top: bool,
) -> Miters {
    let Sides {
        top,
        right,
        bottom,
        left,
    } = border;
    // Each side comes with whether it is the top or the left one, which the
    // shade of a bevel depends on.
    let cut =
        |(first, first_top_left): (&raikiri_style::resolve::ComputedBorder, bool),
         (second, second_top_left): (&raikiri_style::resolve::ComputedBorder, bool)| {
            first.width().px() > 0.0
                && second.width().px() > 0.0
                && !sides_match(
                    first,
                    first_top_left,
                    second,
                    second_top_left,
                    current_color,
                )
        };
    Miters {
        top_left: paint_top && cut((top, true), (left, true)),
        top_right: paint_top && cut((top, true), (right, false)),
        bottom_right: cut((bottom, false), (right, false)),
        bottom_left: cut((bottom, false), (left, true)),
    }
}

/// Whether two sides that meet at a corner are painted alike there, so that no
/// seam shows between them: both have a visible color, and the same one once
/// the shades of a bevel are applied. An `inset` top side and an `outset` right
/// one are both light, and match; an `inset` top and left one are both dark; an
/// `inset` top and right one are not.
fn sides_match(
    first: &raikiri_style::resolve::ComputedBorder,
    first_top_left: bool,
    second: &raikiri_style::resolve::ComputedBorder,
    second_top_left: bool,
    current_color: CssColor,
) -> bool {
    match (
        band_color(first, 0, first_top_left, current_color),
        band_color(second, 0, second_top_left, current_color),
    ) {
        (Some(first), Some(second)) => first == second,
        _ => false,
    }
}

/// The color band `ring` of a side is painted with, `currentcolor` taking the
/// text color, or `None` for a transparent one. `top_left` says whether the side
/// is the top or the left one.
fn band_color(
    side: &raikiri_style::resolve::ComputedBorder,
    ring: usize,
    top_left: bool,
    current_color: CssColor,
) -> Option<CssColor> {
    let color = match side.color {
        raikiri_style::property::BorderColor::Resolved(color) => color,
        _ => current_color,
    };
    if color.a == 0 {
        return None;
    }
    let width = side.width().px() as f64;
    Some(match bevel_dark(side.style(), width, ring, top_left) {
        Some(dark) => bevel::shade(color, dark),
        None => color,
    })
}

/// One band of a side: how far it starts inside the outer edge of the box, and
/// how thick it is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Band {
    offset: f64,
    width: f64,
}

/// The bands of the sides of one ring: where each starts and how thick it is.
struct Ring {
    offsets: Sides<f64>,
    widths: Sides<f64>,
}

/// The two bands of a side `width` thick, the outer one first; the second one
/// has no width for a side that is drawn as a single band.
///
/// A `groove` or `ridge` is cut in two halves, and an odd width leaves the extra
/// pixel to the half that is nearer the top left corner of the box, which is
/// the outer one on the top and left sides (`top_left`) and the inner one on
/// the bottom and right ones. A `double` is two lines of a third each, the
/// middle third being left empty; the third rounds as `(width + 1) / 3` down,
/// so the middle band is a pixel wider or narrower than the others when the
/// width is not a multiple of three. This is how Chromium divides them. Every
/// other style, and a `groove`, `ridge` or `double` that is too thin to divide,
/// is a single band.
fn bands(style: BorderStyle, width: f64, top_left: bool) -> [Band; 2] {
    if bevel::has_two_bands(style, width) {
        let nearer = bevel::nearer_half(width);
        let outer = if top_left { nearer } else { width - nearer };
        [
            Band {
                offset: 0.0,
                width: outer,
            },
            Band {
                offset: outer,
                width: width - outer,
            },
        ]
    } else if bevel::has_two_lines(style, width) {
        let third = bevel::third(width);
        [
            Band {
                offset: 0.0,
                width: third,
            },
            Band {
                offset: width - third,
                width: third,
            },
        ]
    } else {
        [
            Band { offset: 0.0, width },
            Band {
                offset: width,
                width: 0.0,
            },
        ]
    }
}

/// Whether band `ring` of a side in `style` is the dark shade, or `None` when
/// the style is not drawn in shades. An `inset` is dark on the top and left
/// sides and light on the others, and an `outset` the other way round; the outer
/// band of a `groove` is an `inset` and its inner band an `outset`, and a
/// `ridge` the reverse (§3.2).
fn bevel_dark(style: BorderStyle, width: f64, ring: usize, top_left: bool) -> Option<bool> {
    let sunken = match style {
        BorderStyle::Inset => true,
        BorderStyle::Outset => false,
        BorderStyle::Groove | BorderStyle::Ridge if bevel::has_two_bands(style, width) => {
            (style == BorderStyle::Groove) == (ring == 0)
        }
        _ => return None,
    };
    Some(sunken == top_left)
}

/// How one end of a strip meets the strip of the side next to it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Join {
    /// The strip ends square, at the edge of its rectangle.
    Square,
    /// The strip ends in the diagonal of the corner. The diagonal starts at the
    /// outer `corner` of the ring and goes `toward` its inner corner, which is
    /// the widths of the two sides that meet there.
    Miter { corner: Point, toward: Vec2 },
}

/// The strip of one side: the rectangle that is painted, and the extent of the
/// whole side along it, corners included.
#[derive(Clone, Copy)]
struct Strip {
    rect: Rect,
    /// Where the side starts and ends along the strip. A dashed or dotted
    /// pattern is laid out over it from end to end, and only the part inside
    /// `rect` is drawn: a corner that is not cut belongs to the top and bottom
    /// strips.
    line: (f64, f64),
    /// Whether the strip runs along the horizontal axis (the top and bottom
    /// sides) and not the vertical one.
    horizontal: bool,
    /// How the strip ends at its left or top end, and at its right or bottom
    /// end.
    start: Join,
    end: Join,
}

impl Strip {
    /// The top or bottom strip, which spans the whole width of the box.
    fn horizontal(rect: Rect, start: Join, end: Join) -> Self {
        Self {
            rect,
            line: (rect.x0, rect.x1),
            horizontal: true,
            start,
            end,
        }
    }

    /// The same horizontal strip painted only between `from` and `to`, with
    /// its dashes or dots still laid out over the whole side, as where a
    /// vertical side paints the corner square.
    fn within(mut self, from: f64, to: f64) -> Self {
        self.rect.x0 = self.rect.x0.max(from);
        self.rect.x1 = self.rect.x1.min(to);
        self
    }

    /// A left or right strip of a side that is `line` long, between the top and
    /// bottom ones, or over their corners where those are cut.
    fn vertical(rect: Rect, line: (f64, f64), start: Join, end: Join) -> Self {
        Self {
            rect,
            line,
            horizontal: false,
            start,
            end,
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

    /// Whether both ends of the strip are square, so that it is a rectangle.
    fn is_rectangle(&self) -> bool {
        self.start == Join::Square && self.end == Join::Square
    }

    /// Where a `join` ends the strip along its axis, at `across` along the
    /// other one. A square end is at `square`.
    fn end_at(&self, join: Join, across: f64, square: f64) -> f64 {
        let Join::Miter { corner, toward } = join else {
            return square;
        };
        let (low, high) = self.visible();
        // The diagonal from the outer to the inner corner. The side the strip
        // belongs to has a width, so `toward` is not zero across the strip.
        let along = if self.horizontal {
            corner.x + (across - corner.y) * toward.x / toward.y
        } else {
            corner.y + (across - corner.x) * toward.y / toward.x
        };
        along.clamp(low, high)
    }

    /// The part of the strip that lies between `across0` and `across1` across
    /// it, with the ends it has: a rectangle, or a trapezoid where an end is cut.
    fn shape(&self, across0: f64, across1: f64) -> BezPath {
        let (low, high) = self.visible();
        let corner = |across: f64| {
            (
                self.end_at(self.start, across, low),
                self.end_at(self.end, across, high),
            )
        };
        let point = |along: f64, across: f64| {
            if self.horizontal {
                Point::new(along, across)
            } else {
                Point::new(across, along)
            }
        };
        let ((start0, end0), (start1, end1)) = (corner(across0), corner(across1));
        let mut path = BezPath::new();
        path.move_to(point(start0, across0));
        path.line_to(point(end0, across0));
        path.line_to(point(end1, across1));
        path.line_to(point(start1, across1));
        path.close_path();
        path
    }

    /// Fills the part of the strip between `across0` and `across1` across it.
    fn fill_band(&self, scene: &mut impl PaintScene, color: Color, across0: f64, across1: f64) {
        if self.is_rectangle() {
            let Rect { x0, y0, x1, y1 } = self.rect;
            let rect = if self.horizontal {
                Rect::new(x0, across0, x1, across1)
            } else {
                Rect::new(across0, y0, across1, y1)
            };
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
        } else {
            let band = self.shape(across0, across1);
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &band);
        }
    }
}

/// Fills the `strip` of one side in its `style`.
///
/// A `dashed` or `dotted` side that is too short or too long for a pattern is
/// filled solid.
fn fill_strip(scene: &mut impl PaintScene, strip: Strip, color: Color, style: BorderStyle) {
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
    } else {
        strip.fill_band(scene, color, across0, across1);
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
    // A round dot at the end of the visible part sticks out of the strip, and
    // a dash or a dot in a corner that is cut along its diagonal is cut with it.
    let cut = !strip.is_rectangle();
    let clip = cut || (round && (visible0 > start || visible1 < end));
    if cut {
        scene.push_clip_layer(Affine::IDENTITY, &strip.shape(across0, across1));
    } else if clip {
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
pub(super) struct Dashes {
    pub(super) dash: f64,
    pub(super) gap: f64,
    pub(super) count: usize,
}

/// The dashes of a dashed or dotted side `length` long and `thickness` thick,
/// or `None` when the side cannot hold two of them, holds more than
/// [`MAX_SIDE_DASHES`], or has no finite size, and is filled solid.
///
/// A dash is twice the thickness and a gap once from 3px on, three and two times
/// below it; a dot is as long as it is thick and a gap the same. The gap is then
/// stretched or shrunk so that the pattern starts and ends with a dash, whichever
/// of the two nearest dash counts leaves it closest to its natural length. A side
/// that holds just two shrinks the dashes with the gap, and only the gap between
/// two dots. These are the lengths Chromium draws, which matter when a page is
/// compared with its render.
pub(super) fn dash_lengths(length: f64, thickness: f64, dotted: bool) -> Option<Dashes> {
    pattern_lengths(length, thickness, dotted, false)
}

/// The dashes of a closed outline `length` long, such as the centerline of a
/// rounded border, or `None` when it is filled solid instead.
///
/// The pattern starts with a dash and ends with a gap, so that it fits the
/// outline end to end: the gap between two dashes is stretched or shrunk to
/// whichever of the two nearest dash counts leaves it closest to its natural
/// length, as for a side, but an outline has as many gaps as dashes. The square
/// dots of a dotted outline up to 3px thick are not fitted: they keep their
/// size and gap, and the last one is cut off where the outline ends. An outline
/// between two and three dashes long gets two shorter dashes that fill it, and
/// one no longer than two dashes is filled solid.
pub(super) fn closed_dash_lengths(length: f64, thickness: f64, dotted: bool) -> Option<Dashes> {
    pattern_lengths(length, thickness, dotted, true)
}

/// The dashes of a side or of a closed outline; see [`dash_lengths`] and
/// [`closed_dash_lengths`].
fn pattern_lengths(length: f64, thickness: f64, dotted: bool, closed: bool) -> Option<Dashes> {
    let (mut dash, mut gap) = match (dotted, thickness >= 3.0) {
        (true, _) => (thickness, thickness),
        (false, true) => (2.0 * thickness, thickness),
        (false, false) => (3.0 * thickness, 2.0 * thickness),
    };
    if !length.is_finite() || !thickness.is_finite() || length <= 2.0 * dash {
        return None;
    }
    if closed && !dotted && length < 3.0 * dash {
        // Chromium draws two dashes and two gaps, in the ratio of the natural
        // ones, around an outline between two and three dashes long.
        let scale = length / (3.0 * dash);
        dash *= scale;
        gap *= scale;
    }
    if !closed && length <= 2.0 * dash + gap {
        // Exactly two. Dots keep their size and only the gap between them
        // shrinks, so they never reach past the ends of the side; dashes shrink
        // with the gap, in proportion.
        if dotted {
            return Some(Dashes {
                dash,
                gap: length - 2.0 * dash,
                count: 2,
            });
        }
        let scale = length / (2.0 * dash + gap);
        return Some(Dashes {
            dash: dash * scale,
            gap: gap * scale,
            count: 2,
        });
    }
    if closed && dotted && thickness <= 3.0 {
        let count = (length / (dash + gap)).ceil();
        return (count <= MAX_SIDE_DASHES).then_some(Dashes {
            dash,
            gap,
            count: count as usize,
        });
    }
    // A side has one gap fewer than it has dashes, an outline as many.
    let spare = if closed { gap } else { 2.0 * gap };
    let fewer = ((length + spare) / (dash + gap)).floor();
    let more = fewer + 1.0;
    if more > MAX_SIDE_DASHES {
        return None;
    }
    let gap_for = |dashes: f64| {
        let gaps = if closed { dashes } else { dashes - 1.0 };
        (length - dashes * dash) / gaps
    };
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
