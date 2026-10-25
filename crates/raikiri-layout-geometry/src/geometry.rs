use std::ops::{Add, Sub};

/// A width and a height.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Size<T> {
    /// The extent along the horizontal axis.
    pub width: T,
    /// The extent along the vertical axis.
    pub height: T,
}

/// An x and a y coordinate.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Point<T> {
    /// The horizontal coordinate.
    pub x: T,
    /// The vertical coordinate.
    pub y: T,
}

/// A start and an end value.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
#[doc(hidden)]
pub struct Line<T> {
    /// The start value.
    pub start: T,
    /// The end value.
    pub end: T,
}

/// One value for each side of a box.
///
/// This is a set of four edge values, not a rectangle with a position: it holds
/// padding, border and margin widths, or how far scrollable overflow reaches on
/// each side. `left` and `right` are physical sides.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Edges<T> {
    /// The value on the left side.
    pub left: T,
    /// The value on the right side.
    pub right: T,
    /// The value on the top side.
    pub top: T,
    /// The value on the bottom side.
    pub bottom: T,
}

impl<T: Copy> Size<T> {
    /// The size whose width and height are both `value`.
    #[inline]
    pub const fn splat(value: T) -> Size<T> {
        Size {
            width: value,
            height: value,
        }
    }
}

impl<T> Size<T> {
    /// Apply `f` to the width and to the height.
    #[inline]
    pub fn map<R>(self, f: impl Fn(T) -> R) -> Size<R> {
        Size {
            width: f(self.width),
            height: f(self.height),
        }
    }

    /// Combine the widths and the heights of two sizes with `f`.
    #[inline]
    pub fn zip_map<Other, Ret>(self, other: Size<Other>, f: impl Fn(T, Other) -> Ret) -> Size<Ret> {
        Size {
            width: f(self.width, other.width),
            height: f(self.height, other.height),
        }
    }

    /// Swap the width and the height.
    #[inline]
    pub fn transpose(self) -> Size<T> {
        Size {
            width: self.height,
            height: self.width,
        }
    }
}

impl Size<f32> {
    /// Zero width and zero height.
    pub const ZERO: Size<f32> = Size {
        width: 0.0,
        height: 0.0,
    };

    /// The larger width and the larger height of two sizes, by [`f32::max`].
    #[inline]
    pub fn f32_max(self, other: Size<f32>) -> Size<f32> {
        Size {
            width: self.width.max(other.width),
            height: self.height.max(other.height),
        }
    }
}

impl<T> Size<Option<T>> {
    /// The width and the height of `alt` where this size has none.
    #[inline]
    pub fn or(self, alt: Size<Option<T>>) -> Size<Option<T>> {
        Size {
            width: self.width.or(alt.width),
            height: self.height.or(alt.height),
        }
    }
}

impl Size<Option<f32>> {
    /// Neither a width nor a height.
    pub const NONE: Size<Option<f32>> = Size {
        width: None,
        height: None,
    };

    /// Fill in the missing dimension from the other one and `aspect_ratio`
    /// (width divided by height).
    ///
    /// When both dimensions are present, both are missing, or there is no
    /// ratio, the size is returned as it is.
    #[inline]
    pub fn maybe_apply_aspect_ratio(self, aspect_ratio: Option<f32>) -> Size<Option<f32>> {
        match aspect_ratio {
            Some(ratio) => match (self.width, self.height) {
                (Some(width), None) => Size {
                    width: Some(width),
                    height: Some(width / ratio),
                },
                (None, Some(height)) => Size {
                    width: Some(height * ratio),
                    height: Some(height),
                },
                _ => self,
            },
            None => self,
        }
    }
}

impl<U, T: Add<U>> Add<Size<U>> for Size<T> {
    type Output = Size<T::Output>;

    #[inline]
    fn add(self, rhs: Size<U>) -> Self::Output {
        Size {
            width: self.width + rhs.width,
            height: self.height + rhs.height,
        }
    }
}

impl<U, T: Sub<U>> Sub<Size<U>> for Size<T> {
    type Output = Size<T::Output>;

    #[inline]
    fn sub(self, rhs: Size<U>) -> Self::Output {
        Size {
            width: self.width - rhs.width,
            height: self.height - rhs.height,
        }
    }
}

impl<T> Point<T> {
    /// Apply `f` to the x and to the y coordinate.
    #[inline]
    pub fn map<R>(self, f: impl Fn(T) -> R) -> Point<R> {
        Point {
            x: f(self.x),
            y: f(self.y),
        }
    }

    /// Swap the x and the y coordinate.
    #[inline]
    pub fn transpose(self) -> Point<T> {
        Point {
            x: self.y,
            y: self.x,
        }
    }
}

impl Point<f32> {
    /// The origin.
    pub const ZERO: Point<f32> = Point { x: 0.0, y: 0.0 };
}

impl Point<Option<f32>> {
    /// Neither an x nor a y coordinate.
    pub const NONE: Point<Option<f32>> = Point { x: None, y: None };
}

impl<U, T: Add<U>> Add<Point<U>> for Point<T> {
    type Output = Point<T::Output>;

    #[inline]
    fn add(self, rhs: Point<U>) -> Self::Output {
        Point {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl<T> From<Point<T>> for Size<T> {
    /// The x coordinate becomes the width, the y coordinate the height.
    #[inline]
    fn from(value: Point<T>) -> Self {
        Size {
            width: value.x,
            height: value.y,
        }
    }
}

impl<T> Line<T> {
    /// Apply `f` to the start and to the end.
    #[inline]
    pub fn map<R>(self, f: impl Fn(T) -> R) -> Line<R> {
        Line {
            start: f(self.start),
            end: f(self.end),
        }
    }
}

impl Line<bool> {
    /// Both values `true`.
    pub const TRUE: Line<bool> = Line {
        start: true,
        end: true,
    };
    /// Both values `false`.
    pub const FALSE: Line<bool> = Line {
        start: false,
        end: false,
    };
}

impl<T: Copy> Edges<T> {
    /// The edges whose four sides are all `value`.
    #[inline]
    pub const fn splat(value: T) -> Edges<T> {
        Edges {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }
}

impl<T> Edges<T> {
    /// Swap the horizontal and the vertical sides: `left` with `top` and
    /// `right` with `bottom`.
    #[inline]
    pub fn transpose(self) -> Edges<T> {
        Edges {
            left: self.top,
            right: self.bottom,
            top: self.left,
            bottom: self.right,
        }
    }

    /// Apply `f` to all four sides.
    #[inline]
    pub fn map<R>(self, f: impl Fn(T) -> R) -> Edges<R> {
        Edges {
            left: f(self.left),
            right: f(self.right),
            top: f(self.top),
            bottom: f(self.bottom),
        }
    }
}

impl<T: Add<Output = U> + Copy, U> Edges<T> {
    /// The sum of the left and the right side.
    #[inline]
    pub fn horizontal_axis_sum(&self) -> U {
        self.left + self.right
    }

    /// The sum of the top and the bottom side.
    #[inline]
    pub fn vertical_axis_sum(&self) -> U {
        self.top + self.bottom
    }

    /// The horizontal sum as the width and the vertical sum as the height.
    #[inline]
    pub fn sum_axes(&self) -> Size<U> {
        Size {
            width: self.horizontal_axis_sum(),
            height: self.vertical_axis_sum(),
        }
    }
}

impl Edges<f32> {
    /// Zero on every side.
    pub const ZERO: Edges<f32> = Edges {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 0.0,
    };

    /// The edges that reach furthest out on every side: the smaller `left`
    /// and `top`, the larger `right` and `bottom`, by [`f32::min`] and
    /// [`f32::max`].
    ///
    /// The values are read as the coordinates of the sides of a rectangle, so
    /// the result covers both.
    #[inline]
    #[must_use]
    pub fn union(self, other: Edges<f32>) -> Edges<f32> {
        Edges {
            left: self.left.min(other.left),
            right: self.right.max(other.right),
            top: self.top.min(other.top),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

impl Edges<Option<f32>> {
    /// No value on any side.
    pub const NONE: Edges<Option<f32>> = Edges {
        left: None,
        right: None,
        top: None,
        bottom: None,
    };
}

impl<U, T: Add<U>> Add<Edges<U>> for Edges<T> {
    type Output = Edges<T::Output>;

    #[inline]
    fn add(self, rhs: Edges<U>) -> Self::Output {
        Edges {
            left: self.left + rhs.left,
            right: self.right + rhs.right,
            top: self.top + rhs.top,
            bottom: self.bottom + rhs.bottom,
        }
    }
}
