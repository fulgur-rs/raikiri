//! Shared colors, rectangles, insets, and clip geometry.

/// A normalized RGBA color for a neutral paint operation.
///
/// Components are in the inclusive `0.0..=1.0` range by convention. The
/// producer owns conversion from CSS color values; this type has no dependency
/// on a CSS or renderer color representation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintColor {
    /// Red component.
    pub red: f32,
    /// Green component.
    pub green: f32,
    /// Blue component.
    pub blue: f32,
    /// Alpha component.
    pub alpha: f32,
}

impl PaintColor {
    /// Construct a normalized RGBA color.
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

/// A physical CSS-pixel rectangle with its origin in the page box.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintRect {
    /// Physical page-local x coordinate.
    pub x: f32,
    /// Physical page-local y coordinate.
    pub y: f32,
    /// Rectangle width.
    pub width: f32,
    /// Rectangle height.
    pub height: f32,
}

impl PaintRect {
    /// Construct a page-local paint rectangle.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Four-sided CSS-pixel values used by a neutral border operation.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintInsets {
    /// Top value.
    pub top: f32,
    /// Right value.
    pub right: f32,
    /// Bottom value.
    pub bottom: f32,
    /// Left value.
    pub left: f32,
}

impl PaintInsets {
    /// Construct top/right/bottom/left values.
    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }
}

/// A resolved clip operation in page-local CSS pixels.
///
/// An open axis imposes no bound on that axis. Consumers must extend it to
/// their current drawing bounds rather than clipping to the finite rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintClip {
    /// Clip rectangle in page-local coordinates.
    pub rect: PaintRect,
    /// Whether the left and right edges clip the content.
    pub clip_x: bool,
    /// Whether the top and bottom edges clip the content.
    pub clip_y: bool,
    /// Padding-edge ellipses in top-left, top-right, bottom-right, bottom-left
    /// order, each as horizontal and vertical radii. `None` means square corners.
    ///
    /// Curves apply only when both axes clip. The producer resolves percentages
    /// and overlap on the whole border box before subtracting border widths.
    /// A radius can exceed this rectangle when an opposite border crops the
    /// curve; consumers must not normalize the radii to the inner rectangle.
    pub corner_radii: Option<[[f32; 2]; 4]>,
}

impl PaintClip {
    /// Construct a square-cornered clip to `rect` on both axes.
    pub const fn new(rect: PaintRect) -> Self {
        Self {
            rect,
            clip_x: true,
            clip_y: true,
            corner_radii: None,
        }
    }
}

/// A 2D affine transform between CSS-pixel coordinate spaces.
///
/// The six values are those of CSS `matrix(a, b, c, d, e, f)`: the point
/// `(x, y)` maps to `(a·x + c·y + e, b·x + d·y + f)`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintTransform {
    /// Horizontal scale.
    pub a: f32,
    /// Vertical skew.
    pub b: f32,
    /// Horizontal skew.
    pub c: f32,
    /// Vertical scale.
    pub d: f32,
    /// Horizontal translation.
    pub e: f32,
    /// Vertical translation.
    pub f: f32,
}

impl PaintTransform {
    /// The transform that maps every point to itself.
    pub const IDENTITY: Self = Self::new(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);

    /// Construct a transform from its matrix values.
    pub const fn new(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self { a, b, c, d, e, f }
    }

    /// A translation by `(x, y)`.
    pub const fn translate(x: f32, y: f32) -> Self {
        Self::new(1.0, 0.0, 0.0, 1.0, x, y)
    }

    /// Where `point` lands.
    pub fn apply(self, point: (f32, f32)) -> (f32, f32) {
        (
            self.a * point.0 + self.c * point.1 + self.e,
            self.b * point.0 + self.d * point.1 + self.f,
        )
    }
}

/// `outer * inner` applies `inner` first and then `outer`, like the product
/// of their matrices.
impl std::ops::Mul for PaintTransform {
    type Output = Self;

    fn mul(self, inner: Self) -> Self {
        Self::new(
            self.a * inner.a + self.c * inner.b,
            self.b * inner.a + self.d * inner.b,
            self.a * inner.c + self.c * inner.d,
            self.b * inner.c + self.d * inner.d,
            self.a * inner.e + self.c * inner.f + self.e,
            self.b * inner.e + self.d * inner.f + self.f,
        )
    }
}
