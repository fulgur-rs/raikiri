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
