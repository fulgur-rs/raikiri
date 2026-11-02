//! Warnings for the lengths of a box that Taffy cannot take as they are.
//!
//! The style views hand layout every length clamped to the range
//! [`MAX_TAFFY_MAGNITUDE`] with NaN taken as zero. Author CSS can still carry
//! such values (`1e40px`, `0 * 1e40em`), and a clamp that changes one is
//! reported as a [`LayoutWarn::NonFiniteClamped`] naming the property. The
//! bridge checks the lengths of each element once, in the order of the
//! properties listed here.

use super::*;
use crate::layout::sanitize::sanitize_taffy;

/// Reports `value` of the property `site` if clamping changes it.
fn check(value: f32, site: &'static str, diag: &mut Vec<LayoutWarn>) {
    sanitize_taffy(value, site, diag);
}

/// The lengths and percentages that a `<length-percentage> | auto` value
/// hands Taffy: percentages as fractions.
fn check_length_percentage_or_auto(
    value: &ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) {
    match value {
        ComputedLengthPercentageOrAuto::Px(px) => check(*px, site, diag),
        ComputedLengthPercentageOrAuto::Percent(percent) => check(percent / 100.0, site, diag),
        _ => {}
    }
}

/// Like [`check_length_percentage_or_auto`], for a preferred size, whose
/// `fit-content()` limit is handed to Taffy as well.
fn check_size(
    value: &ComputedLengthPercentageOrAuto,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) {
    match value {
        ComputedLengthPercentageOrAuto::FitContentOf(ComputedLengthPercentageOrCalc::Px(px)) => {
            check(*px, site, diag);
        }
        ComputedLengthPercentageOrAuto::FitContentOf(ComputedLengthPercentageOrCalc::Percent(
            percent,
        )) => check(percent / 100.0, site, diag),
        value => check_length_percentage_or_auto(value, site, diag),
    }
}

/// The sizes, insets, margins, padding and size limits of `cv`.
///
/// `vertical_axes` tells whether the box lays out on vertical axes, which
/// decides what its minimum height is ([`physical_min_height`]).
pub(super) fn report_non_finite_sizes(
    cv: &ComputedValues,
    vertical_axes: bool,
    diag: &mut Vec<LayoutWarn>,
) {
    check_size(&cv.width, "width", diag);
    check_size(&cv.height, "height", diag);
    for (value, site) in [
        (&cv.top, "top"),
        (&cv.right, "right"),
        (&cv.bottom, "bottom"),
        (&cv.left, "left"),
    ] {
        check_length_percentage_or_auto(value, site, diag);
    }
    for margin in [
        &cv.margin.top,
        &cv.margin.right,
        &cv.margin.bottom,
        &cv.margin.left,
    ] {
        check_length_percentage_or_auto(margin, "margin", diag);
    }
    for padding in [
        cv.padding.top,
        cv.padding.right,
        cv.padding.bottom,
        cv.padding.left,
    ] {
        match padding {
            ComputedLengthPercentageOrCalc::Px(px) => check(px, "padding", diag),
            ComputedLengthPercentageOrCalc::Percent(percent) => {
                check(percent / 100.0, "padding", diag);
            }
            ComputedLengthPercentageOrCalc::Calc(_) => {}
        }
    }
    for (value, site) in [
        (&cv.min_width, "min-width"),
        (physical_min_height(cv, vertical_axes), "min-height"),
        (&cv.max_width, "max-width"),
        (&cv.max_height, "max-height"),
    ] {
        check_length_percentage_or_auto(value, site, diag);
    }
}

/// The border widths, flex basis, gaps and grid tracks of `cv`.
pub(super) fn report_non_finite_rest(cv: &ComputedValues, diag: &mut Vec<LayoutWarn>) {
    taffy_border_widths(&cv.border, diag);
    match cv.flex_basis {
        ComputedFlexBasis::Px(px) => check(px, "flex-basis", diag),
        ComputedFlexBasis::Percent(percent) => check(percent / 100.0, "flex-basis", diag),
        _ => {}
    }
    for (gap, site) in [(cv.column_gap, "column-gap"), (cv.row_gap, "row-gap")] {
        match gap {
            ComputedLengthPercentageOrNormal::Px(px) => check(px, site, diag),
            ComputedLengthPercentageOrNormal::Percent(percent) => {
                check(percent / 100.0, site, diag);
            }
            ComputedLengthPercentageOrNormal::Normal
            | ComputedLengthPercentageOrNormal::Calc(_) => {}
        }
    }
    for (tracks, site) in [
        (&cv.grid_template_columns, "grid-template-columns"),
        (&cv.grid_template_rows, "grid-template-rows"),
    ] {
        let ComputedGridTemplateTracks::List(list) = tracks else {
            continue;
        };
        for component in &list.components {
            match component {
                ComputedGridTrackListComponent::Size(size) => check_track(*size, site, diag),
                ComputedGridTrackListComponent::Repeat(repeat) => {
                    for size in &repeat.tracks {
                        check_track(*size, site, diag);
                    }
                }
            }
        }
    }
    for (tracks, site) in [
        (&cv.grid_auto_columns, "grid-auto-columns"),
        (&cv.grid_auto_rows, "grid-auto-rows"),
    ] {
        for size in tracks.iter() {
            check_track(*size, site, diag);
        }
    }
}

/// The lengths of one track sizing function: the minimum side is checked
/// before the maximum one, and a plain breadth is both.
fn check_track(size: ComputedGridTrackSize, site: &'static str, diag: &mut Vec<LayoutWarn>) {
    use ComputedGridTrackBreadth as B;
    let check_min = |breadth, diag: &mut Vec<LayoutWarn>| match breadth {
        B::Px(px) => check(px, site, diag),
        B::Percent(percent) => check(percent / 100.0, site, diag),
        _ => {}
    };
    let check_max = |breadth, diag: &mut Vec<LayoutWarn>| match breadth {
        B::Px(px) => check(px, site, diag),
        B::Percent(percent) => check(percent / 100.0, site, diag),
        B::Flex(fraction) => check(fraction, site, diag),
        _ => {}
    };
    match size {
        ComputedGridTrackSize::Breadth(breadth) => {
            check_min(breadth, diag);
            check_max(breadth, diag);
        }
        ComputedGridTrackSize::MinMax(min, max) => {
            check_min(min, diag);
            check_max(max, diag);
        }
        ComputedGridTrackSize::FitContent(limit) => match limit {
            ComputedLengthPercentage::Px(px) => check(px, site, diag),
            ComputedLengthPercentage::Percent(percent) => check(percent / 100.0, site, diag),
        },
    }
}
