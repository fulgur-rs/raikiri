//! Resolving style lengths against a percentage basis.
//!
//! A length that holds a `calc()` resolves through the
//! [`CalcRef`](crate::values::CalcRef) it carries, so the callers of these
//! traits pass no function to dereference a handle. The traits also resolve the
//! sizes and edges of lengths.

use crate::values::{Dimension, LengthPercentage, LengthPercentageAuto};
use raikiri_layout_geometry::{Edges, Size};

/// Resolving a length that may depend on its containing block, where a length
/// that cannot be resolved gives `None`.
#[doc(hidden)]
pub trait MaybeResolve<In, Out> {
    /// The absolute length, or `None` if it is `auto`, an intrinsic keyword, or
    /// a percentage of a `context` that is not known.
    fn maybe_resolve(self, context: In) -> Out;
}

/// Resolving a length that may depend on its containing block, where a length
/// that cannot be resolved gives zero.
#[doc(hidden)]
pub trait ResolveOrZero<In, Out> {
    /// The absolute length, or zero where it cannot be resolved.
    fn resolve_or_zero(self, context: In) -> Out;
}

macro_rules! resolve_style_length {
    ($($length:ty),* $(,)?) => {$(
        impl MaybeResolve<Option<f32>, Option<f32>> for $length {
            #[inline]
            fn maybe_resolve(self, context: Option<f32>) -> Option<f32> {
                self.resolve(context)
            }
        }

        impl MaybeResolve<f32, Option<f32>> for $length {
            #[inline]
            fn maybe_resolve(self, context: f32) -> Option<f32> {
                self.resolve(Some(context))
            }
        }

        impl ResolveOrZero<Option<f32>, f32> for $length {
            #[inline]
            fn resolve_or_zero(self, context: Option<f32>) -> f32 {
                self.resolve(context).unwrap_or(0.0)
            }
        }
    )*};
}

resolve_style_length!(Dimension, LengthPercentage, LengthPercentageAuto);

impl<In, Out, T: MaybeResolve<In, Out>> MaybeResolve<Size<In>, Size<Out>> for Size<T> {
    #[inline(always)]
    fn maybe_resolve(self, context: Size<In>) -> Size<Out> {
        Size {
            width: self.width.maybe_resolve(context.width),
            height: self.height.maybe_resolve(context.height),
        }
    }
}

impl<T: ResolveOrZero<Option<f32>, f32>> ResolveOrZero<Size<Option<f32>>, Size<f32>> for Size<T> {
    #[inline]
    fn resolve_or_zero(self, context: Size<Option<f32>>) -> Size<f32> {
        Size {
            width: self.width.resolve_or_zero(context.width),
            height: self.height.resolve_or_zero(context.height),
        }
    }
}

impl<T: ResolveOrZero<Option<f32>, f32>> ResolveOrZero<Size<Option<f32>>, Edges<f32>> for Edges<T> {
    #[inline]
    fn resolve_or_zero(self, context: Size<Option<f32>>) -> Edges<f32> {
        Edges {
            left: self.left.resolve_or_zero(context.width),
            right: self.right.resolve_or_zero(context.width),
            top: self.top.resolve_or_zero(context.height),
            bottom: self.bottom.resolve_or_zero(context.height),
        }
    }
}

impl<T: ResolveOrZero<Option<f32>, f32>> ResolveOrZero<Option<f32>, Edges<f32>> for Edges<T> {
    #[inline(always)]
    fn resolve_or_zero(self, context: Option<f32>) -> Edges<f32> {
        Edges {
            left: self.left.resolve_or_zero(context),
            right: self.right.resolve_or_zero(context),
            top: self.top.resolve_or_zero(context),
            bottom: self.bottom.resolve_or_zero(context),
        }
    }
}
