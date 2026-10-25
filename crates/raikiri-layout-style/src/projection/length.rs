//! Style lengths.
//!
//! A length keeps the layout library's compact form, which is eight bytes
//! wide, so that handing it to the library costs nothing. The form is private
//! to the projection: a length is made with the constructors here, read with
//! `expand`, and resolved with the methods here. The rest of the crate and its
//! users see only that surface.

use crate::values::{
    CalcRef, ExpandedDimension, ExpandedLengthPercentage, ExpandedLengthPercentageAuto,
    LengthIdentity,
};
use taffy::CompactLength;

/// A size: a length, a percentage, a `calc()`, `auto` or an intrinsic sizing
/// keyword.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dimension(pub(super) taffy::Dimension);

/// A length, a percentage or a `calc()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthPercentage(pub(super) taffy::LengthPercentage);

/// A length, a percentage, a `calc()` or `auto`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthPercentageAuto(pub(super) taffy::LengthPercentageAuto);

impl Dimension {
    /// An absolute length.
    #[inline(always)]
    pub const fn length(value: f32) -> Self {
        Self(taffy::Dimension::length(value))
    }

    /// A percentage, as a fraction of one.
    #[inline(always)]
    pub const fn percent(value: f32) -> Self {
        Self(taffy::Dimension::percent(value))
    }

    /// `auto`.
    #[inline(always)]
    pub const fn auto() -> Self {
        Self(taffy::Dimension::auto())
    }

    /// `min-content`.
    #[inline(always)]
    pub const fn min_content() -> Self {
        Self(taffy::Dimension::min_content())
    }

    /// `max-content`.
    #[inline(always)]
    pub const fn max_content() -> Self {
        Self(taffy::Dimension::max_content())
    }

    /// `fit-content`.
    #[inline(always)]
    pub const fn fit_content() -> Self {
        Self(taffy::Dimension::fit_content())
    }

    /// `fit-content(<length>)`.
    #[inline(always)]
    pub const fn fit_content_px(limit: f32) -> Self {
        Self(taffy::Dimension::fit_content_px(limit))
    }

    /// `fit-content(<percentage>)`, as a fraction of one.
    #[inline(always)]
    pub const fn fit_content_percent(limit: f32) -> Self {
        Self(taffy::Dimension::fit_content_percent(limit))
    }

    /// `stretch`.
    #[inline(always)]
    pub const fn stretch() -> Self {
        Self(taffy::Dimension::stretch())
    }

    /// `content`.
    #[inline(always)]
    pub const fn content() -> Self {
        Self(taffy::Dimension::content())
    }

    /// A `calc()`.
    #[inline]
    pub fn calc(calc: CalcRef) -> Self {
        Self(taffy::Dimension::calc(calc.handle()))
    }

    /// `fit-content(<calc()>)`.
    #[inline]
    pub fn fit_content_calc(calc: CalcRef) -> Self {
        Self(taffy::Dimension::fit_content_calc(calc.handle()))
    }

    /// Whether the dimension is `auto`.
    #[inline(always)]
    pub fn is_auto(self) -> bool {
        self.0.is_auto()
    }

    /// The dimension as a value to match on.
    pub fn expand(self) -> ExpandedDimension {
        match self.0.expand() {
            taffy::ExpandedDimension::Length(value) => ExpandedDimension::Length(value),
            taffy::ExpandedDimension::Percent(value) => ExpandedDimension::Percent(value),
            taffy::ExpandedDimension::Auto => ExpandedDimension::Auto,
            taffy::ExpandedDimension::MinContent => ExpandedDimension::MinContent,
            taffy::ExpandedDimension::MaxContent => ExpandedDimension::MaxContent,
            taffy::ExpandedDimension::FitContentPx(value) => ExpandedDimension::FitContentPx(value),
            taffy::ExpandedDimension::FitContentPercent(value) => {
                ExpandedDimension::FitContentPercent(value)
            }
            taffy::ExpandedDimension::FitContentCalc(handle) => {
                ExpandedDimension::FitContentCalc(CalcRef::from_handle(handle))
            }
            taffy::ExpandedDimension::FitContent => ExpandedDimension::FitContent,
            taffy::ExpandedDimension::Stretch => ExpandedDimension::Stretch,
            taffy::ExpandedDimension::Content => ExpandedDimension::Content,
            taffy::ExpandedDimension::Calc(handle) => {
                ExpandedDimension::Calc(CalcRef::from_handle(handle))
            }
        }
    }
}

impl LengthPercentage {
    /// A length of zero.
    pub const ZERO: Self = Self(taffy::LengthPercentage::length(0.0));

    /// An absolute length.
    #[inline(always)]
    pub const fn length(value: f32) -> Self {
        Self(taffy::LengthPercentage::length(value))
    }

    /// A percentage, as a fraction of one.
    #[inline(always)]
    pub const fn percent(value: f32) -> Self {
        Self(taffy::LengthPercentage::percent(value))
    }

    /// A `calc()`.
    #[inline]
    pub fn calc(calc: CalcRef) -> Self {
        Self(taffy::LengthPercentage::calc(calc.handle()))
    }

    /// The length as a value to match on.
    pub fn expand(self) -> ExpandedLengthPercentage {
        match self.0.expand() {
            taffy::ExpandedLengthPercentage::Length(value) => {
                ExpandedLengthPercentage::Length(value)
            }
            taffy::ExpandedLengthPercentage::Percent(value) => {
                ExpandedLengthPercentage::Percent(value)
            }
            taffy::ExpandedLengthPercentage::Calc(handle) => {
                ExpandedLengthPercentage::Calc(CalcRef::from_handle(handle))
            }
        }
    }
}

impl LengthPercentageAuto {
    /// A length of zero.
    pub const ZERO: Self = Self(taffy::LengthPercentageAuto::length(0.0));

    /// An absolute length.
    #[inline(always)]
    pub const fn length(value: f32) -> Self {
        Self(taffy::LengthPercentageAuto::length(value))
    }

    /// A percentage, as a fraction of one.
    #[inline(always)]
    pub const fn percent(value: f32) -> Self {
        Self(taffy::LengthPercentageAuto::percent(value))
    }

    /// `auto`.
    #[inline(always)]
    pub const fn auto() -> Self {
        Self(taffy::LengthPercentageAuto::auto())
    }

    /// A `calc()`.
    #[inline]
    pub fn calc(calc: CalcRef) -> Self {
        Self(taffy::LengthPercentageAuto::calc(calc.handle()))
    }

    /// Whether the length is `auto`.
    #[inline(always)]
    pub fn is_auto(self) -> bool {
        self.0.is_auto()
    }

    /// The length as a value to match on.
    pub fn expand(self) -> ExpandedLengthPercentageAuto {
        match self.0.expand() {
            taffy::ExpandedLengthPercentageAuto::Length(value) => {
                ExpandedLengthPercentageAuto::Length(value)
            }
            taffy::ExpandedLengthPercentageAuto::Percent(value) => {
                ExpandedLengthPercentageAuto::Percent(value)
            }
            taffy::ExpandedLengthPercentageAuto::Auto => ExpandedLengthPercentageAuto::Auto,
            taffy::ExpandedLengthPercentageAuto::Calc(handle) => {
                ExpandedLengthPercentageAuto::Calc(CalcRef::from_handle(handle))
            }
        }
    }
}

impl From<LengthPercentageAuto> for Dimension {
    #[inline]
    fn from(length: LengthPercentageAuto) -> Self {
        Self(length.0.into())
    }
}

/// The identity of a length: its kind and value, or the payload of a `calc()`.
fn identity_of(raw: CompactLength) -> LengthIdentity {
    let value = if raw.has_calc() {
        CalcRef::from_handle(raw.calc_value()).addr()
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
    LengthIdentity::new(raw.tag(), value)
}

macro_rules! resolve_length {
    ($($length:ident),+ $(,)?) => {$(
        impl $length {
            /// The absolute length, resolving a percentage or a `calc()` against
            /// `basis`.
            ///
            /// `None` for `auto` and the intrinsic keywords, and for a percentage
            /// or `calc()` when there is no basis. A plain length keeps a
            /// non-finite value.
            #[inline]
            pub fn resolve(self, basis: Option<f32>) -> Option<f32> {
                let raw = self.0.into_raw();
                // A calc handle fills the bits above the tag with its identity, so inspect it first.
                if raw.is_plain_calc() {
                    return basis.map(|basis| CalcRef::from_handle(raw.calc_value()).resolve(basis));
                }
                match raw.tag() {
                    CompactLength::LENGTH_TAG => Some(raw.value()),
                    CompactLength::PERCENT_TAG => basis.map(|basis| raw.value() * basis),
                    _ => None,
                }
            }

            /// Whether the length is a `calc()` that is not a `fit-content()` limit.
            #[inline]
            pub fn is_calc(self) -> bool {
                self.0.into_raw().is_plain_calc()
            }

            /// Whether the length depends on the size of its containing block.
            #[inline]
            pub fn uses_percentage(self) -> bool {
                self.0.into_raw().uses_percentage()
            }

            /// The kind and value of the length, for use in a cache key.
            #[inline]
            pub fn identity(self) -> LengthIdentity {
                identity_of(self.0.into_raw())
            }
        }
    )+};
}

resolve_length!(Dimension, LengthPercentage, LengthPercentageAuto);

// The layout library keeps a `calc()` length as a raw pointer in its compact
// form, which makes the three length types `!Send`, and with them every style
// store and document that holds one. The pointer is never dereferenced: it is
// the handle of a slot of the calc table (see `calc_slab`), and resolving it
// reads only that table, which synchronizes its own accesses. A length handed
// to another thread therefore names the same slot there, and a handle whose
// payload is gone resolves to nothing. These impls are the one place that says
// so, and a type that holds a length is `Send` only when everything else it
// holds is, which the compiler checks.
//
// The claim depends on the table staying one process-wide, synchronized
// structure whose readers reject a slot that was freed or reused. If a table
// ever became thread-local or per document, or a reader skipped the state
// check, these impls would have to go. `tests/stale_calc.rs` moves each length
// to another thread and fails in the first case.
//
// `Sync` is not claimed; nothing shares a length across threads by reference.
#[allow(unsafe_code)]
unsafe impl Send for Dimension {}
#[allow(unsafe_code)]
unsafe impl Send for LengthPercentage {}
#[allow(unsafe_code)]
unsafe impl Send for LengthPercentageAuto {}
