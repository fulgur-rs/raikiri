use crate::calc_slab::Slot;
use crate::{Axis, LengthProperty};
use raikiri_style::property::CalcLengthPercentage;

/// A calc expression with the range policy of the property it belongs to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CalcValue {
    expression: CalcLengthPercentage,
    non_negative: bool,
}

impl CalcValue {
    pub(crate) const fn new(expression: CalcLengthPercentage, non_negative: bool) -> Self {
        Self {
            expression,
            non_negative,
        }
    }

    /// The expression resolved against a CSS percentage basis, with its range
    /// applied.
    pub(crate) fn resolve(self, basis: f32) -> f32 {
        let value = basis * self.expression.percent / 100.0 + self.expression.px;
        if !value.is_finite() {
            0.0
        } else if self.non_negative {
            value.max(0.0)
        } else {
            value
        }
    }

    /// Whether the expression has a percentage term, which needs a basis.
    pub(crate) fn has_percentage(self) -> bool {
        self.expression.percent != 0.0
    }

    pub(crate) const fn non_negative(self) -> bool {
        self.non_negative
    }

    /// The bits of the expression, `px` above `percent`.
    pub(crate) fn to_bits(self) -> u64 {
        (u64::from(self.expression.px.to_bits()) << 32)
            | u64::from(self.expression.percent.to_bits())
    }

    /// The value whose expression is `bits` (see [`Self::to_bits`]).
    pub(crate) fn from_bits(bits: u64, non_negative: bool) -> Self {
        let expression = CalcLengthPercentage {
            px: f32::from_bits((bits >> 32) as u32),
            percent: f32::from_bits(bits as u32),
        };
        Self::new(expression, non_negative)
    }
}

/// Immutable payload of a `calc()` length.
///
/// The payload owns a slot of the table that its handle names, so a length is
/// resolved through the payload only while the payload is alive: the handle of
/// a dropped payload resolves to nothing instead of dangling.
#[derive(Debug)]
#[doc(hidden)]
pub struct AlignedCalc {
    value: CalcValue,
    slot: Slot,
}
impl AlignedCalc {
    /// Return the immutable length/percentage expression.
    pub const fn expression(&self) -> CalcLengthPercentage {
        self.value.expression
    }
    /// Whether resolution clamps the result to the non-negative range.
    pub const fn non_negative(&self) -> bool {
        self.value.non_negative
    }
    /// Construct a payload with the property's range policy.
    pub fn new(value: CalcLengthPercentage, non_negative: bool) -> Self {
        let value = CalcValue::new(value, non_negative);
        Self {
            value,
            slot: Slot::new(value),
        }
    }
    /// Resolve against a CSS percentage basis, applying range checking.
    pub fn resolve(&self, basis: f32) -> f32 {
        self.value.resolve(basis)
    }
    /// Resolve `expression` against a CSS percentage basis without making a
    /// payload, applying the range policy `non_negative` selects.
    pub fn resolve_expression(
        expression: CalcLengthPercentage,
        non_negative: bool,
        basis: f32,
    ) -> f32 {
        CalcValue::new(expression, non_negative).resolve(basis)
    }
    pub(crate) fn matches(&self, value: CalcLengthPercentage, non_negative: bool) -> bool {
        self.value.expression.px.to_bits() == value.px.to_bits()
            && self.value.expression.percent.to_bits() == value.percent.to_bits()
            && self.value.non_negative == non_negative
    }
    /// The handle that names this payload in the table.
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
