//! Resolution of style lengths against an optional percentage basis.
//!
//! Calc handles must belong to a live document layout arena. Without a basis,
//! only a plain length resolves; callers choose any context-specific fallback
//! basis and apply their own finite-value or range policy.

use raikiri_layout_style::values::{Dimension, LengthExt, LengthPercentage, LengthPercentageAuto};

/// Resolve a length or percentage, preserving non-finite plain values.
pub(crate) fn resolve_length_percentage(
    value: LengthPercentage,
    basis: Option<f32>,
) -> Option<f32> {
    value.resolve(basis)
}

/// Resolve a length or percentage, returning `None` for auto.
pub(crate) fn resolve_length_percentage_auto(
    value: LengthPercentageAuto,
    basis: Option<f32>,
) -> Option<f32> {
    value.resolve(basis)
}

/// Resolve a dimension, preserving non-finite plain values.
pub(crate) fn resolve_dimension(value: Dimension, basis: Option<f32>) -> Option<f32> {
    value.resolve(basis)
}
