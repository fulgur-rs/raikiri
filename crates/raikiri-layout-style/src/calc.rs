use crate::calc_slab::Slot;
use crate::{Axis, LengthProperty};
use raikiri_style::{
    ComputedLengthPercentageOrAuto as L,
    property::{CalcLengthPercentage, ComputedLengthMath},
};
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

/// An affine calc or an owning comparison tree, prepared before layout queries.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub enum CalcExpression {
    /// An inline absolute/percentage pair.
    Affine(CalcLengthPercentage),
    /// A comparison retained until its percentage basis is known.
    Math(Arc<ComputedLengthMath>),
}
impl From<CalcLengthPercentage> for CalcExpression {
    fn from(value: CalcLengthPercentage) -> Self {
        Self::Affine(value)
    }
}
impl From<Arc<ComputedLengthMath>> for CalcExpression {
    fn from(value: Arc<ComputedLengthMath>) -> Self {
        Self::Math(value)
    }
}
impl From<&CalcExpression> for CalcExpression {
    fn from(value: &CalcExpression) -> Self {
        value.clone()
    }
}
impl PartialEq for CalcExpression {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Affine(a), Self::Affine(b)) => {
                a.px.to_bits() == b.px.to_bits() && a.percent.to_bits() == b.percent.to_bits()
            }
            (Self::Math(a), Self::Math(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for CalcExpression {}
impl Hash for CalcExpression {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Affine(value) => {
                0u8.hash(state);
                value.px.to_bits().hash(state);
                value.percent.to_bits().hash(state);
            }
            Self::Math(value) => {
                1u8.hash(state);
                value.hash(state);
            }
        }
    }
}
impl CalcExpression {
    /// An owning computed value, constructed during preparation or mutation.
    pub fn as_computed(&self) -> L {
        match self {
            Self::Affine(value) => L::Calc(*value),
            Self::Math(value) => L::Math(value.clone()),
        }
    }
}

/// A prepared expression and the range policy of its property.
#[derive(Clone, Debug)]
pub(crate) struct CalcValue {
    expression: CalcExpression,
    non_negative: bool,
}
impl CalcValue {
    pub(crate) fn new(expression: impl Into<CalcExpression>, non_negative: bool) -> Self {
        Self {
            expression: expression.into(),
            non_negative,
        }
    }
    pub(crate) fn resolve(&self, basis: f32) -> f32 {
        let value = match &self.expression {
            CalcExpression::Affine(value) => basis * value.percent / 100.0 + value.px,
            CalcExpression::Math(value) => value.resolve(basis),
        };
        range(value, self.non_negative)
    }
    pub(crate) fn has_percentage(&self) -> bool {
        match &self.expression {
            CalcExpression::Affine(value) => value.percent != 0.0,
            CalcExpression::Math(value) => value.has_percentage(),
        }
    }
    pub(crate) const fn non_negative(&self) -> bool {
        self.non_negative
    }
    pub(crate) fn math(&self) -> Option<&Arc<ComputedLengthMath>> {
        match &self.expression {
            CalcExpression::Math(value) => Some(value),
            _ => None,
        }
    }
    /// The affine coefficient bits, or none for an owning comparison.
    pub(crate) fn to_bits(&self) -> Option<u64> {
        match self.expression {
            CalcExpression::Affine(value) => {
                Some((u64::from(value.px.to_bits()) << 32) | u64::from(value.percent.to_bits()))
            }
            CalcExpression::Math(_) => None,
        }
    }
    pub(crate) fn from_bits(bits: u64, non_negative: bool) -> Self {
        Self::new(
            CalcLengthPercentage {
                px: f32::from_bits((bits >> 32) as u32),
                percent: f32::from_bits(bits as u32),
            },
            non_negative,
        )
    }
}
fn range(value: f32, non_negative: bool) -> f32 {
    if !value.is_finite() {
        0.0
    } else if non_negative {
        value.max(0.0)
    } else {
        value
    }
}

/// Immutable payload retained while its checked handle is in use.
/// The projected value is built once so queries can borrow it without cloning
/// the tree. For a comparison it retains a second Arc to the same expression.
#[derive(Debug)]
#[doc(hidden)]
pub struct AlignedCalc {
    value: CalcValue,
    slot: Slot,
    projected: L,
}
impl AlignedCalc {
    /// The immutable expression, without cloning its owner.
    pub fn expression(&self) -> &CalcExpression {
        &self.value.expression
    }
    /// Whether the final result is clamped to the non-negative range.
    pub const fn non_negative(&self) -> bool {
        self.value.non_negative
    }
    /// Construct a payload with the property's range policy.
    pub fn new(value: impl Into<CalcExpression>, non_negative: bool) -> Self {
        let value = CalcValue::new(value, non_negative);
        let projected = value.expression.as_computed();
        let slot = Slot::new(value.clone());
        Self {
            value,
            slot,
            projected,
        }
    }
    pub(crate) fn projected(&self) -> &L {
        &self.projected
    }
    /// Resolve the whole expression, then apply its range.
    pub fn resolve(&self, basis: f32) -> f32 {
        self.value.resolve(basis)
    }
    /// Resolve an affine value without allocating a payload.
    pub fn resolve_expression(
        expression: CalcLengthPercentage,
        non_negative: bool,
        basis: f32,
    ) -> f32 {
        CalcValue::new(expression, non_negative).resolve(basis)
    }
    /// Resolve a borrowed comparison without constructing or cloning an owner.
    pub fn resolve_math_expression(
        expression: &ComputedLengthMath,
        non_negative: bool,
        basis: f32,
    ) -> f32 {
        range(expression.resolve(basis), non_negative)
    }
    pub(crate) fn matches(&self, value: &CalcExpression, non_negative: bool) -> bool {
        self.expression() == value && self.non_negative() == non_negative
    }
    pub(crate) fn handle(&self) -> usize {
        self.slot.handle()
    }
}

/// Which Grid list contains a calc expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum GridList {
    /// Explicit template components and repeats.
    Template,
    /// Implicit tracks.
    Auto,
}

/// Which half of a track sizing function contains a calc expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum TrackPart {
    /// The minimum breadth.
    Min,
    /// The maximum breadth.
    Max,
}

/// Stable property/component identity for a prepared payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum CalcSlot {
    /// A scalar property.
    Property(LengthProperty),
    /// A Grid track breadth.
    Grid {
        /// Columns or rows.
        axis: Axis,
        /// Explicit or implicit tracks.
        list: GridList,
        /// Top-level component index.
        component: usize,
        /// Track index inside a repeat, if any.
        repeat_track: Option<usize>,
        /// Minimum or maximum breadth.
        part: TrackPart,
    },
}

impl CalcSlot {
    pub(crate) fn search_key(self) -> (u8, u8, u8, usize, Option<usize>, u8) {
        match self {
            Self::Property(property) => (0, 0, 0, property as usize, None, 0),
            Self::Grid {
                axis,
                list,
                component,
                repeat_track,
                part,
            } => (
                1,
                axis as u8,
                list as u8,
                component,
                repeat_track,
                part as u8,
            ),
        }
    }
}

/// A payload borrowed from an owner that outlives all uses of the view.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct PreparedCalc<'a> {
    /// Property/component identity.
    pub slot: CalcSlot,
    /// Immutable payload.
    pub payload: &'a AlignedCalc,
}
