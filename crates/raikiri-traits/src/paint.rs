//! Shared colors, rectangles, insets, and rectangular clips.

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

/// A rectangular clip operation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PaintClip {
    /// Clip rectangle in page-local coordinates.
    pub rect: PaintRect,
}
