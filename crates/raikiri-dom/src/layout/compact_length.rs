//! Resolution of Taffy's compact style lengths.

use taffy::{CompactLength, Dimension, LengthPercentage, LengthPercentageAuto};

/// Resolve a compact length against an optional percentage basis.
/// Calc handles must belong to a live document layout arena. Without a
/// basis, only a plain length resolves; callers choose any context-specific
/// fallback basis and apply their own finite-value or range policy.
fn resolve_compact(raw: CompactLength, basis: Option<f32>) -> Option<f32> {
    // Calc pointers carry address bits in the tag, so inspect them first.
    if raw.is_calc() {
        return basis.map(|basis| crate::taffy_impl::resolve_calc(raw.calc_value(), basis));
    }
    match raw.tag() {
        CompactLength::LENGTH_TAG => Some(raw.value()),
        CompactLength::PERCENT_TAG => basis.map(|basis| raw.value() * basis),
        _ => None,
    }
}

/// Resolve a length or percentage, preserving non-finite plain values.
pub(crate) fn resolve_length_percentage(
    value: LengthPercentage,
    basis: Option<f32>,
) -> Option<f32> {
    resolve_compact(value.into_raw(), basis)
}

/// Resolve a length or percentage, returning `None` for auto.
pub(crate) fn resolve_length_percentage_auto(
    value: LengthPercentageAuto,
    basis: Option<f32>,
) -> Option<f32> {
    resolve_compact(value.into_raw(), basis)
}

/// Resolve a dimension, preserving non-finite plain values.
pub(crate) fn resolve_dimension(value: Dimension, basis: Option<f32>) -> Option<f32> {
    resolve_compact(value.into_raw(), basis)
}
