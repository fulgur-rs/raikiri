//! Resolving style lengths against the geometry the layout produces.
//!
//! A style length (`Dimension`, `LengthPercentage`, `LengthPercentageAuto`) and
//! the size or edges of them are the library's style vocabulary. Resolving one
//! against a containing block takes the layout's [`Size`] as the containing
//! block and gives the layout's [`Size`] or [`Edges`] back. The traits here
//! have the same methods as the library's of the same names, so that the layout
//! code can call them on the style values without handing the library a type of
//! its own geometry.

use raikiri_layout_geometry::{Edges, Size};
use taffy::{Dimension, LengthPercentage, LengthPercentageAuto};

/// The four sides of a style value, as the layout's [`Edges`].
pub(crate) trait IntoEdges<T> {
    /// The same values on the same sides.
    fn into_edges(self) -> Edges<T>;
}

impl<T> IntoEdges<T> for taffy::Rect<T> {
    #[inline(always)]
    fn into_edges(self) -> Edges<T> {
        let taffy::Rect {
            left,
            right,
            top,
            bottom,
        } = self;
        Edges {
            left,
            right,
            top,
            bottom,
        }
    }
}

/// Resolving a length that may depend on its containing block, where a length
/// that cannot be resolved gives `None`.
pub(crate) trait MaybeResolve<In, Out> {
    /// The absolute length, or `None` if it is `auto`, an intrinsic keyword, or
    /// a percentage of a `context` that is not known.
    fn maybe_resolve(self, context: In, calc: impl Fn(*const (), f32) -> f32) -> Out;
}

/// Resolving a length that may depend on its containing block, where a length
/// that cannot be resolved gives zero.
pub(crate) trait ResolveOrZero<In, Out> {
    /// The absolute length, or zero where it cannot be resolved.
    fn resolve_or_zero(self, context: In, calc: impl Fn(*const (), f32) -> f32) -> Out;
}

macro_rules! resolve_style_length {
    ($($length:ty),* $(,)?) => {$(
        impl MaybeResolve<Option<f32>, Option<f32>> for $length {
            #[inline]
            fn maybe_resolve(
                self,
                context: Option<f32>,
                calc: impl Fn(*const (), f32) -> f32,
            ) -> Option<f32> {
                taffy::MaybeResolve::maybe_resolve(self, context, calc)
            }
        }

        impl MaybeResolve<f32, Option<f32>> for $length {
            #[inline]
            fn maybe_resolve(
                self,
                context: f32,
                calc: impl Fn(*const (), f32) -> f32,
            ) -> Option<f32> {
                taffy::MaybeResolve::maybe_resolve(self, Some(context), calc)
            }
        }

        impl ResolveOrZero<Option<f32>, f32> for $length {
            #[inline]
            fn resolve_or_zero(
                self,
                context: Option<f32>,
                calc: impl Fn(*const (), f32) -> f32,
            ) -> f32 {
                taffy::ResolveOrZero::resolve_or_zero(self, context, calc)
            }
        }
    )*};
}

resolve_style_length!(Dimension, LengthPercentage, LengthPercentageAuto);

impl<In, Out, T: MaybeResolve<In, Out>> MaybeResolve<Size<In>, Size<Out>> for taffy::Size<T> {
    #[inline(always)]
    fn maybe_resolve(self, context: Size<In>, calc: impl Fn(*const (), f32) -> f32) -> Size<Out> {
        Size {
            width: self.width.maybe_resolve(context.width, &calc),
            height: self.height.maybe_resolve(context.height, &calc),
        }
    }
}

impl<T: ResolveOrZero<Option<f32>, f32>> ResolveOrZero<Size<Option<f32>>, Size<f32>>
    for taffy::Size<T>
{
    #[inline]
    fn resolve_or_zero(
        self,
        context: Size<Option<f32>>,
        calc: impl Fn(*const (), f32) -> f32,
    ) -> Size<f32> {
        Size {
            width: self.width.resolve_or_zero(context.width, &calc),
            height: self.height.resolve_or_zero(context.height, &calc),
        }
    }
}

impl<T: ResolveOrZero<Option<f32>, f32>> ResolveOrZero<Size<Option<f32>>, Edges<f32>>
    for taffy::Rect<T>
{
    #[inline]
    fn resolve_or_zero(
        self,
        context: Size<Option<f32>>,
        calc: impl Fn(*const (), f32) -> f32,
    ) -> Edges<f32> {
        Edges {
            left: self.left.resolve_or_zero(context.width, &calc),
            right: self.right.resolve_or_zero(context.width, &calc),
            top: self.top.resolve_or_zero(context.height, &calc),
            bottom: self.bottom.resolve_or_zero(context.height, &calc),
        }
    }
}

impl<T: ResolveOrZero<Option<f32>, f32>> ResolveOrZero<Option<f32>, Edges<f32>> for taffy::Rect<T> {
    #[inline(always)]
    fn resolve_or_zero(
        self,
        context: Option<f32>,
        calc: impl Fn(*const (), f32) -> f32,
    ) -> Edges<f32> {
        Edges {
            left: self.left.resolve_or_zero(context, &calc),
            right: self.right.resolve_or_zero(context, &calc),
            top: self.top.resolve_or_zero(context, &calc),
            bottom: self.bottom.resolve_or_zero(context, &calc),
        }
    }
}
