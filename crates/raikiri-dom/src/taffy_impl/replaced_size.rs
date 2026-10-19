//! The size of a replaced leaf with an intrinsic ratio and an `auto` side.

use super::{calc_has_percentage, resolve_calc};
use crate::image_geometry::constrain_with_ratio;
use taffy::{
    BoxSizing, Dimension, ExpandedDimension, ExpandedLengthPercentageAuto, LayoutInput,
    LengthPercentageAuto, MaybeResolve, ResolveOrZero, Size, SizingMode, Style,
};

/// The content-box size a replaced leaf takes when `width`, `height` or both
/// are `auto` and the caller fixes at most the width, or `None` when Taffy's
/// own leaf sizing applies.
///
/// With both sides `auto`, the natural size is brought within the `min-*` and
/// `max-*` sizes by the constraint table of CSS 2.1 §10.4. With one side
/// authored, the other follows from the ratio (CSS 2.1 §10.3.2, §10.6.2) of the
/// authored side after its own `min-*` and `max-*`, and is then clamped by its
/// own bounds (§10.4, §10.7). A width the caller has already fixed, say the one
/// an earlier measurement of the same box produced, gives the height the same
/// way. Taffy's leaf sizing would instead floor the height at the width divided
/// by the ratio, which undoes a `max-height` and keeps the height of an image
/// whose `max-width` shrank its width.
///
/// A side the author fixed is not decided here: Taffy takes its authored size
/// over the one this function derives for it.
///
/// A percentage that has no definite basis resolves to nothing, as for any
/// other box. The sizes are resolved to the content box, so that
/// `box-sizing: border-box` bounds the border box.
pub(super) fn auto_sides_size(
    style: &Style,
    inputs: &LayoutInput,
    natural: Size<f32>,
) -> Option<Size<f32>> {
    let ratio = style
        .aspect_ratio
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)?;
    let valid = |side: f32| side.is_finite() && side > 0.0;
    if inputs.sizing_mode != SizingMode::InherentSize
        || inputs.known_dimensions.height.is_some()
        || !valid(natural.width)
        || !valid(natural.height)
    {
        return None;
    }
    let basis = inputs.parent_size;
    let edges = (style.padding.resolve_or_zero(basis.width, resolve_calc)
        + style.border.resolve_or_zero(basis.width, resolve_calc))
    .sum_axes();
    let content = |size: Size<Option<f32>>| Size {
        width: size
            .width
            .map(|side| content_side(style, side, edges.width)),
        height: size
            .height
            .map(|side| content_side(style, side, edges.height)),
    };
    let min = content(Size {
        width: resolve_bound(style.min_size.width, basis.width, true),
        height: resolve_bound(style.min_size.height, basis.height, true),
    });
    let max = content(Size {
        width: resolve_bound(
            style.max_size.width,
            basis.width,
            inputs.available_space.width == taffy::AvailableSpace::MinContent,
        ),
        height: resolve_bound(
            style.max_size.height,
            basis.height,
            inputs.available_space.height == taffy::AvailableSpace::MinContent,
        ),
    });
    let authored = content(Size {
        width: resolve_cyclic_size(
            style.size.width,
            basis.width,
            inputs.available_space.width == taffy::AvailableSpace::MinContent,
        ),
        height: resolve_cyclic_size(
            style.size.height,
            basis.height,
            inputs.available_space.height == taffy::AvailableSpace::MinContent,
        ),
    });
    let (min_width, min_height) = (min.width.unwrap_or(0.0), min.height.unwrap_or(0.0));
    // A max below its min is raised to it.
    let max_width = max.width.unwrap_or(f32::INFINITY).max(min_width);
    let max_height = max.height.unwrap_or(f32::INFINITY).max(min_height);
    let clamp_width = |width: f32| width.min(max_width).max(min_width);
    let clamp_height = |height: f32| height.min(max_height).max(min_height);
    if let Some(fixed) = inputs.known_dimensions.width {
        // The caller's width is a border-box width and is not clamped again.
        let width = (fixed - edges.width).max(0.0);
        return Some(Size {
            width,
            height: clamp_height(width / ratio),
        });
    }
    let (width, height) = match (authored.width, authored.height) {
        (Some(width), _) => {
            let width = clamp_width(width);
            (width, clamp_height(width / ratio))
        }
        (None, Some(height)) => {
            let height = clamp_height(height);
            (clamp_width(height * ratio), height)
        }
        (None, None) => constrain_with_ratio(
            (natural.width, natural.height),
            (min_width, max_width),
            (min_height, max_height),
        ),
    };
    Some(Size { width, height })
}

/// Resolve cyclic percentages in intrinsic sizing inputs for a replaced box.
fn resolve_cyclic_size(value: Dimension, basis: Option<f32>, min_content: bool) -> Option<f32> {
    if min_content && basis.is_none() {
        return match value.expand() {
            ExpandedDimension::Percent(_) => Some(0.0),
            ExpandedDimension::Calc(handle) if calc_has_percentage(handle) => {
                Some(resolve_calc(handle, 0.0))
            }
            _ => value.maybe_resolve(basis, resolve_calc),
        };
    }
    value.maybe_resolve(basis, resolve_calc)
}

/// A cyclic percentage in a replaced box's minimum size resolves against zero
/// for both min-content and max-content contributions.
fn resolve_bound(
    value: LengthPercentageAuto,
    basis: Option<f32>,
    min_content: bool,
) -> Option<f32> {
    if min_content && basis.is_none() {
        return match value.expand() {
            ExpandedLengthPercentageAuto::Percent(_) => Some(0.0),
            ExpandedLengthPercentageAuto::Calc(handle) if calc_has_percentage(handle) => {
                Some(resolve_calc(handle, 0.0))
            }
            _ => value.maybe_resolve(basis, resolve_calc),
        };
    }
    value.maybe_resolve(basis, resolve_calc)
}

/// A side of a box resolved against the box `style` sizes: the side itself for
/// `content-box`, and the side less the padding and border for `border-box`.
fn content_side(style: &Style, side: f32, edges: f32) -> f32 {
    match style.box_sizing {
        BoxSizing::ContentBox => side,
        BoxSizing::BorderBox => (side - edges).max(0.0),
    }
}

#[cfg(test)]
mod tests;
