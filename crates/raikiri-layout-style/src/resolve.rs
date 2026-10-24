//! Resolving style lengths against a percentage basis.
//!
//! A style length (`Dimension`, `LengthPercentage`, `LengthPercentageAuto`) can
//! carry a handle to a `calc()` payload that the view's getters publish. The
//! traits here resolve lengths, and the sizes and edges of them, without the
//! caller passing a function that dereferences such a handle.
//!
//! # Calc handles
//!
//! A handle must come from a getter of this crate, and the owner of its
//! payload must stay alive for as long as the handle is resolved. The
//! adapter retains every payload for the lifetime of its session before it
//! publishes the handle, so any length read from a view meets this. A length
//! that did not come from a view must not hold a calc handle.

use crate::AlignedCalc;
use raikiri_layout_geometry::{Edges, Size};
use taffy::{CompactLength, Dimension, LengthPercentage, LengthPercentageAuto};

/// The single place that turns a calc handle back into its payload.
///
/// The lifetime is unbounded because the handle does not carry one; the
/// module documentation states the contract that makes it sound.
#[allow(unsafe_code)]
pub(crate) fn payload<'a>(handle: *const ()) -> &'a AlignedCalc {
    // SAFETY: Handles are produced by `AlignedCalc::pointer` for payloads that
    // the adapter retains before publishing them (see the module documentation).
    unsafe { &*handle.cast::<AlignedCalc>() }
}

/// Reading a calc payload through the handle Taffy's lengths carry.
pub trait CalcHandle: Copy {
    /// Resolve the payload against the percentage `basis`, applying its range check.
    fn resolve(self, basis: f32) -> f32;

    /// Whether the payload has a percentage term, which needs a basis to resolve.
    fn has_percentage(self) -> bool;
}

impl CalcHandle for *const () {
    #[inline]
    fn resolve(self, basis: f32) -> f32 {
        payload(self).resolve(basis)
    }

    #[inline]
    fn has_percentage(self) -> bool {
        payload(self).expression().percent != 0.0
    }
}

/// What distinguishes one style length from another, for a cache key.
///
/// Two lengths have the same identity when they are the same kind with the
/// same value; a calc is identified by its payload, not by its value. A zero
/// value has one identity whatever its sign, and every NaN has one identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LengthIdentity {
    tag: usize,
    value: usize,
}

impl LengthIdentity {
    fn of(raw: CompactLength) -> Self {
        let value = if raw.has_calc() {
            raw.calc_value().addr()
        } else {
            let value = raw.value();
            if value.is_nan() {
                f32::NAN.to_bits() as usize
            } else if value == 0.0 {
                0
            } else {
                value.to_bits() as usize
            }
        };
        Self {
            tag: raw.tag(),
            value,
        }
    }
}

/// Questions about a style length that do not need to look at its encoding.
pub trait LengthExt: Copy {
    /// The absolute length, resolving a percentage or a `calc()` against
    /// `basis`.
    ///
    /// `None` for `auto` and the intrinsic keywords, and for a percentage or
    /// `calc()` when there is no basis. A plain length keeps a non-finite value.
    fn resolve(self, basis: Option<f32>) -> Option<f32>;

    /// Whether the length is a `calc()` that is not a `fit-content()` limit.
    fn is_calc(self) -> bool;

    /// Whether the length depends on the size of its containing block.
    fn uses_percentage(self) -> bool;

    /// The kind and value of the length, for use in a cache key.
    fn identity(self) -> LengthIdentity;
}

macro_rules! length_ext {
    ($($length:ty),+ $(,)?) => {$(
        impl LengthExt for $length {
            #[inline]
            fn resolve(self, basis: Option<f32>) -> Option<f32> {
                let raw = self.into_raw();
                // Calc pointers carry address bits in the tag, so inspect them first.
                if raw.is_plain_calc() {
                    return basis.map(|basis| raw.calc_value().resolve(basis));
                }
                match raw.tag() {
                    CompactLength::LENGTH_TAG => Some(raw.value()),
                    CompactLength::PERCENT_TAG => basis.map(|basis| raw.value() * basis),
                    _ => None,
                }
            }

            #[inline]
            fn is_calc(self) -> bool {
                self.into_raw().is_plain_calc()
            }

            #[inline]
            fn uses_percentage(self) -> bool {
                self.into_raw().uses_percentage()
            }

            #[inline]
            fn identity(self) -> LengthIdentity {
                LengthIdentity::of(self.into_raw())
            }
        }
    )+};
}

length_ext!(Dimension, LengthPercentage, LengthPercentageAuto);

/// The four sides of a style value, as the layout's [`Edges`].
pub trait IntoEdges<T> {
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
pub trait MaybeResolve<In, Out> {
    /// The absolute length, or `None` if it is `auto`, an intrinsic keyword, or
    /// a percentage of a `context` that is not known.
    fn maybe_resolve(self, context: In) -> Out;
}

/// Resolving a length that may depend on its containing block, where a length
/// that cannot be resolved gives zero.
pub trait ResolveOrZero<In, Out> {
    /// The absolute length, or zero where it cannot be resolved.
    fn resolve_or_zero(self, context: In) -> Out;
}

macro_rules! resolve_style_length {
    ($($length:ty),* $(,)?) => {$(
        impl MaybeResolve<Option<f32>, Option<f32>> for $length {
            #[inline]
            fn maybe_resolve(self, context: Option<f32>) -> Option<f32> {
                taffy::MaybeResolve::maybe_resolve(self, context, <*const () as CalcHandle>::resolve)
            }
        }

        impl MaybeResolve<f32, Option<f32>> for $length {
            #[inline]
            fn maybe_resolve(self, context: f32) -> Option<f32> {
                taffy::MaybeResolve::maybe_resolve(self, Some(context), <*const () as CalcHandle>::resolve)
            }
        }

        impl ResolveOrZero<Option<f32>, f32> for $length {
            #[inline]
            fn resolve_or_zero(self, context: Option<f32>) -> f32 {
                taffy::ResolveOrZero::resolve_or_zero(self, context, <*const () as CalcHandle>::resolve)
            }
        }
    )*};
}

resolve_style_length!(Dimension, LengthPercentage, LengthPercentageAuto);

impl<In, Out, T: MaybeResolve<In, Out>> MaybeResolve<Size<In>, Size<Out>> for taffy::Size<T> {
    #[inline(always)]
    fn maybe_resolve(self, context: Size<In>) -> Size<Out> {
        Size {
            width: self.width.maybe_resolve(context.width),
            height: self.height.maybe_resolve(context.height),
        }
    }
}

impl<T: ResolveOrZero<Option<f32>, f32>> ResolveOrZero<Size<Option<f32>>, Size<f32>>
    for taffy::Size<T>
{
    #[inline]
    fn resolve_or_zero(self, context: Size<Option<f32>>) -> Size<f32> {
        Size {
            width: self.width.resolve_or_zero(context.width),
            height: self.height.resolve_or_zero(context.height),
        }
    }
}

impl<T: ResolveOrZero<Option<f32>, f32>> ResolveOrZero<Size<Option<f32>>, Edges<f32>>
    for taffy::Rect<T>
{
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

impl<T: ResolveOrZero<Option<f32>, f32>> ResolveOrZero<Option<f32>, Edges<f32>> for taffy::Rect<T> {
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
