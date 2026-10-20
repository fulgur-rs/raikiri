use crate::{Axis, LengthProperty};
use raikiri_style::property::CalcLengthPercentage;

/// Immutable payload retained by the adapter before a view is constructed.
#[repr(align(8))]
#[derive(Debug)]
pub struct AlignedCalc {
    value: CalcLengthPercentage,
    non_negative: bool,
}
const _: () = assert!(std::mem::align_of::<AlignedCalc>() >= 8);
impl AlignedCalc {
    pub(crate) fn expression(&self) -> CalcLengthPercentage {
        self.value
    }
    pub(crate) fn non_negative(&self) -> bool {
        self.non_negative
    }
    /// Construct a payload with the property's range policy.
    pub const fn new(value: CalcLengthPercentage, non_negative: bool) -> Self {
        Self {
            value,
            non_negative,
        }
    }
    /// Resolve against a CSS percentage basis, applying range checking.
    pub fn resolve(&self, basis: f32) -> f32 {
        let value = basis * self.value.percent / 100.0 + self.value.px;
        if !value.is_finite() {
            0.0
        } else if self.non_negative {
            value.max(0.0)
        } else {
            value
        }
    }
    pub(crate) fn matches(&self, value: CalcLengthPercentage, non_negative: bool) -> bool {
        self.value.px.to_bits() == value.px.to_bits()
            && self.value.percent.to_bits() == value.percent.to_bits()
            && self.non_negative == non_negative
    }
    pub(crate) fn pointer(&self) -> *const () {
        std::ptr::from_ref(self).cast()
    }
}

/// Which Grid list contains a calc expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridList {
    /// Explicit template components and repeats.
    Template,
    /// Implicit tracks.
    Auto,
}

/// Which half of a track sizing function contains a calc expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackPart {
    /// The minimum breadth.
    Min,
    /// The maximum breadth.
    Max,
}

/// Stable property/component identity for a prepared payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// A payload borrowed from an owner that outlives all uses of the view.
#[derive(Clone, Copy, Debug)]
pub struct PreparedCalc<'a> {
    /// Property/component identity.
    pub slot: CalcSlot,
    /// Immutable, aligned payload.
    pub payload: &'a AlignedCalc,
}

/// Resolve a raw Taffy calc handle produced by this crate.
///
/// # Safety
/// The pointer must come from a getter of this crate, refer to a live
/// AlignedCalc, and its owner must remain alive throughout this call.
/// A Taffy length is Copy and does not carry that lifetime; retaining the
/// length after dropping its prepared owner does not permit resolving it.
#[allow(unsafe_code)]
pub unsafe fn resolve_calc(pointer: *const (), basis: f32) -> f32 {
    // SAFETY: The caller guarantees origin, alignment, liveness and immutable access.
    unsafe { &*pointer.cast::<AlignedCalc>() }.resolve(basis)
}
