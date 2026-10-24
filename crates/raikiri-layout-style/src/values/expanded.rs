//! Style lengths as values to match on, and the identity of a length.

use super::calc_ref::CalcRef;

/// A [`Dimension`](super::Dimension) as a value to match on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExpandedDimension {
    /// An absolute length.
    Length(f32),
    /// A percentage, as a fraction of one.
    Percent(f32),
    /// `auto`.
    Auto,
    /// `min-content`.
    MinContent,
    /// `max-content`.
    MaxContent,
    /// `fit-content(<length>)`.
    FitContentPx(f32),
    /// `fit-content(<percentage>)`, as a fraction of one.
    FitContentPercent(f32),
    /// `fit-content(<calc()>)`.
    FitContentCalc(CalcRef),
    /// `fit-content`.
    FitContent,
    /// `stretch`.
    Stretch,
    /// `content`.
    Content,
    /// A `calc()`.
    Calc(CalcRef),
}

/// A [`LengthPercentage`](super::LengthPercentage) as a value to match on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExpandedLengthPercentage {
    /// An absolute length.
    Length(f32),
    /// A percentage, as a fraction of one.
    Percent(f32),
    /// A `calc()`.
    Calc(CalcRef),
}

/// A [`LengthPercentageAuto`](super::LengthPercentageAuto) as a value to match on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExpandedLengthPercentageAuto {
    /// An absolute length.
    Length(f32),
    /// A percentage, as a fraction of one.
    Percent(f32),
    /// `auto`.
    Auto,
    /// A `calc()`.
    Calc(CalcRef),
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
    pub(crate) fn new(tag: usize, value: usize) -> Self {
        Self { tag, value }
    }
}
