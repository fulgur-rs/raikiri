//! The square border of a box, painted as one strip per side (CSS Backgrounds
//! 3 §5).

use super::*;

/// The square border of one box. A `double` side draws its outer and inner
/// thirds and leaves the middle one transparent (§5.6); every other style that
/// has a color fills the whole strip.
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
        if self.paint_top
            && bt > 0.0
            && let Some(color) = self.color(&self.border.top)
        {
            let strip = Rect::new(x0, y0, x1, y0 + bt);
            fill_strip(scene, strip, bt, true, color, is_double(&self.border.top));
        }
        if bb > 0.0
            && let Some(color) = self.color(&self.border.bottom)
        {
            let strip = Rect::new(x0, y1 - bb, x1, y1);
            fill_strip(
                scene,
                strip,
                bb,
                true,
                color,
                is_double(&self.border.bottom),
            );
        }
        if bl > 0.0
            && inner_height_valid
            && let Some(color) = self.color(&self.border.left)
        {
            let strip = Rect::new(x0, inner_y0, x0 + bl, inner_y1);
            fill_strip(scene, strip, bl, false, color, is_double(&self.border.left));
        }
        if br > 0.0
            && inner_height_valid
            && let Some(color) = self.color(&self.border.right)
        {
            let strip = Rect::new(x1 - br, inner_y0, x1, inner_y1);
            fill_strip(
                scene,
                strip,
                br,
                false,
                color,
                is_double(&self.border.right),
            );
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

fn is_double(side: &raikiri_style::resolve::ComputedBorder) -> bool {
    side.style() == BorderStyle::Double
}

/// Fills the strip `rect` of one side, `thickness` thick. `double` draws the
/// outer and inner thirds of the thickness (§5.6, "two parallel solid
/// lines"), along the strip when it is `horizontal` and across it otherwise.
/// Below 3px the thirds vanish, so a small double falls back to solid, which
/// matches the clamping of browsers closely enough for reftests.
fn fill_strip(
    scene: &mut impl PaintScene,
    rect: Rect,
    thickness: f64,
    horizontal: bool,
    color: Color,
    double: bool,
) {
    let Rect { x0, y0, x1, y1 } = rect;
    if !double || thickness < 3.0 {
        scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
        return;
    }
    let third = thickness / 3.0;
    let (outer, inner) = if horizontal {
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
}

#[cfg(test)]
mod tests;
