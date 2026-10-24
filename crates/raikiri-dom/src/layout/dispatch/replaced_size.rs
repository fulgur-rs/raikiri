//! The size of a replaced leaf with an intrinsic ratio and an `auto` side.

use crate::LayoutStyle as Style;
use crate::image_geometry::constrain_with_ratio;
use crate::layout::dispatch::content_limits::ContentLimits;
use crate::layout::protocol::{AvailableSpace, LayoutInput, SizingMode};
use raikiri_layout_geometry::Size;
use raikiri_layout_style::values::{
    BoxSizing, CalcHandle, Dimension, ExpandedDimension, ExpandedLengthPercentageAuto,
    LengthPercentageAuto, MaybeResolve, ResolveOrZero,
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
///
/// `content_box_ratio` says that the ratio relates content box sizes whatever
/// the `box-sizing` is, as the natural ratio of the content does. Otherwise a
/// `border-box` box has the ratio of its border box (CSS Box Sizing 4 §5.1),
/// and the padding and border take part in the sizes it gives across.
pub(super) fn auto_sides_size(
    style: &Style,
    inputs: &LayoutInput,
    natural: Size<f32>,
    limits: ContentLimits,
    content_box_ratio: bool,
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
    let is_min_content = inputs.available_space.width == AvailableSpace::MinContent
        || inputs.available_space.height == AvailableSpace::MinContent;
    let edges = padding_and_border(style, basis.width);
    let insets = ratio_box_insets(style, edges, content_box_ratio);
    let height_for = |width: f32| height_for_width(width, ratio, insets);
    let width_for = |height: f32| width_for_height(height, ratio, insets);
    let content = |size: Size<Option<f32>>| Size {
        width: size
            .width
            .map(|side| content_side(style, side, edges.width)),
        height: size
            .height
            .map(|side| content_side(style, side, edges.height)),
    };
    let mut min = content(Size {
        width: resolve_bound(style.min_size.width, basis.width, true),
        height: resolve_bound(style.min_size.height, basis.height, true),
    });
    let mut max = content(Size {
        width: resolve_bound(style.max_size.width, basis.width, is_min_content),
        height: resolve_bound(style.max_size.height, basis.height, is_min_content),
    });
    let authored = content(Size {
        width: resolve_cyclic_size(style.size.width, basis.width, is_min_content),
        height: resolve_cyclic_size(style.size.height, basis.height, is_min_content),
    });
    let keyword = keyword_bounds(style, inputs, natural, limits, content_box_ratio);
    if let Some(keyword) = keyword {
        min.width = merge_min(
            min.width,
            keyword
                .width
                .min
                .map(|size| content_box_from_border_box(size, edges.width)),
        );
        min.height = merge_min(
            min.height,
            keyword
                .height
                .min
                .map(|size| content_box_from_border_box(size, edges.height)),
        );
        max.width = merge_max(
            max.width,
            keyword
                .width
                .max
                .map(|size| content_box_from_border_box(size, edges.width)),
        );
        max.height = merge_max(
            max.height,
            keyword
                .height
                .max
                .map(|size| content_box_from_border_box(size, edges.height)),
        );
    } // cov:ignore: valid natural dimensions above guarantee keyword_bounds returns Some.
    let (min_width, min_height) = (min.width.unwrap_or(0.0), min.height.unwrap_or(0.0));
    // A max below its min is raised to it.
    let max_width = max.width.unwrap_or(f32::INFINITY).max(min_width);
    let max_height = max.height.unwrap_or(f32::INFINITY).max(min_height);
    let clamp_width = |width: f32| width.min(max_width).max(min_width);
    let clamp_height = |height: f32| height.min(max_height).max(min_height);
    if let Some(fixed) = inputs.known_dimensions.width {
        // The caller's width is not clamped by numeric sizes here. Intrinsic
        // keyword bounds transfer across the ratio, though, and can constrain
        // the size the parent offers the replaced item.
        let width = keyword.map_or((fixed - edges.width).max(0.0), |bounds| {
            let min = bounds
                .width
                .min
                .map(|size| content_box_from_border_box(size, edges.width));
            let max = bounds
                .width
                .max
                .map(|size| content_box_from_border_box(size, edges.width));
            size_in_bounds((fixed - edges.width).max(0.0), min, max)
        });
        return Some(Size {
            width,
            height: clamp_height(height_for(width)),
        });
    }
    let (width, height) = match (authored.width, authored.height) {
        (Some(width), Some(height)) => (clamp_width(width), clamp_height(height)),
        (Some(width), _) => {
            let width = clamp_width(width);
            (width, clamp_height(height_for(width)))
        }
        (None, Some(height)) => {
            let height = clamp_height(height);
            (clamp_width(width_for(height)), height)
        }
        (None, None) => {
            // The natural width stays. The natural height does too when the
            // ratio is the natural one; a ratio the box states replaces it,
            // and the height follows the width along it (CSS 2.1 §10.6.2).
            let natural_height = if content_box_ratio {
                natural.height
            } else {
                height_for(natural.width)
            };
            let (width, height) = constrain_with_ratio(
                (natural.width + insets.width, natural_height + insets.height),
                (min_width + insets.width, max_width + insets.width),
                (min_height + insets.height, max_height + insets.height),
            );
            (
                (width - insets.width).max(0.0),
                (height - insets.height).max(0.0),
            )
        }
    };
    Some(Size { width, height })
}

/// Border-box bounds of the intrinsic sizing keywords on a replaced element.
///
/// Replaced content has the same natural min-content and max-content size. A
/// definite opposite side may transfer that contribution through the
/// preferred ratio. The numeric bounds of the opposite side constrain the size
/// that is transferred, but the numeric bounds of the side the keyword is on do
/// not limit the result: a `min-width: max-content` takes the width the height
/// gives, whatever `max-width` says.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct KeywordBounds {
    pub width: AxisBounds,
    pub height: AxisBounds,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct AxisBounds {
    pub min: Option<f32>,
    pub max: Option<f32>,
}

/// Resolve the intrinsic keyword sizes of a replaced element in border-box
/// coordinates. `None` means the node has no valid natural dimensions.
pub(super) fn keyword_bounds(
    style: &Style,
    inputs: &LayoutInput,
    natural: Size<f32>,
    limits: ContentLimits,
    content_box_ratio: bool,
) -> Option<KeywordBounds> {
    let ratio = style
        .aspect_ratio
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0);
    let valid = |side: f32| side.is_finite() && side > 0.0;
    if !valid(natural.width) || !valid(natural.height) {
        return None;
    }

    let basis = inputs.parent_size;
    let is_min_content = inputs.available_space.width == AvailableSpace::MinContent
        || inputs.available_space.height == AvailableSpace::MinContent;
    let edges = padding_and_border(style, basis.width);
    let insets = ratio_box_insets(style, edges, content_box_ratio);
    let min_content = Size {
        width: resolve_bound(style.min_size.width, basis.width, is_min_content)
            .map(|size| content_side(style, size, edges.width)),
        height: resolve_bound(style.min_size.height, basis.height, is_min_content)
            .map(|size| content_side(style, size, edges.height)),
    };
    let max_content = Size {
        width: resolve_bound(style.max_size.width, basis.width, is_min_content)
            .map(|size| content_side(style, size, edges.width)),
        height: resolve_bound(style.max_size.height, basis.height, is_min_content)
            .map(|size| content_side(style, size, edges.height)),
    };
    let authored = Size {
        width: resolve_cyclic_size(style.size.width, basis.width, is_min_content)
            .map(|size| content_side(style, size, edges.width)),
        height: resolve_cyclic_size(style.size.height, basis.height, is_min_content)
            .map(|size| content_side(style, size, edges.height)),
    };
    let clamp = |size: f32, min: Option<f32>, max: Option<f32>| {
        size.min(max.unwrap_or(f32::INFINITY).max(min.unwrap_or(0.0)))
            .max(min.unwrap_or(0.0))
    };
    let preferred_width = clamp(
        authored.width.unwrap_or(natural.width),
        min_content.width,
        max_content.width,
    );
    let preferred_height = clamp(
        authored.height.unwrap_or(natural.height),
        min_content.height,
        max_content.height,
    );

    // A size transferred across the ratio is not limited by the numeric bounds
    // of the side it lands on.
    let contribution_width = match (ratio, authored.height) {
        (Some(ratio), Some(_)) => width_for_height(preferred_height, ratio, insets),
        _ => clamp(natural.width, min_content.width, max_content.width),
    };
    let contribution_height = match (ratio, authored.width) {
        (Some(ratio), Some(_)) => height_for_width(preferred_width, ratio, insets),
        _ => clamp(natural.height, min_content.height, max_content.height),
    };

    let resolve_keyword = |dimension: Dimension, contribution: f32| -> Option<f32> {
        let expanded = dimension.expand();
        matches!(
            expanded,
            ExpandedDimension::MinContent
                | ExpandedDimension::MaxContent
                | ExpandedDimension::FitContent
                | ExpandedDimension::FitContentPx(_)
                | ExpandedDimension::FitContentPercent(_)
                | ExpandedDimension::FitContentCalc(_)
        )
        .then_some(contribution)
    };

    let mut bounds = KeywordBounds {
        width: AxisBounds {
            min: resolve_keyword(limits.min.width, contribution_width)
                .map(|size| size + edges.width),
            max: resolve_keyword(limits.max.width, contribution_width)
                .map(|size| size + edges.width),
        },
        height: AxisBounds {
            min: resolve_keyword(limits.min.height, contribution_height)
                .map(|size| size + edges.height),
            max: resolve_keyword(limits.max.height, contribution_height)
                .map(|size| size + edges.height),
        },
    };
    if let Some(ratio) = ratio {
        bounds.width.min = merge_min(
            bounds.width.min,
            bounds.height.min.map(|height| {
                width_for_height((height - edges.height).max(0.0), ratio, insets) + edges.width
            }),
        );
        bounds.width.max = merge_max(
            bounds.width.max,
            bounds.height.max.map(|height| {
                width_for_height((height - edges.height).max(0.0), ratio, insets) + edges.width
            }),
        );
        bounds.height.min = merge_min(
            bounds.height.min,
            bounds.width.min.map(|width| {
                height_for_width((width - edges.width).max(0.0), ratio, insets) + edges.height
            }),
        );
        bounds.height.max = merge_max(
            bounds.height.max,
            bounds.width.max.map(|width| {
                height_for_width((width - edges.width).max(0.0), ratio, insets) + edges.height
            }),
        );
    } // cov:ignore: llvm-cov reports the covered ratio-transfer block's closing brace as uncovered.
    Some(bounds)
}

fn size_in_bounds(size: f32, min: Option<f32>, max: Option<f32>) -> f32 {
    size.min(max.unwrap_or(f32::INFINITY).max(min.unwrap_or(0.0)))
        .max(min.unwrap_or(0.0))
}

fn merge_min(left: Option<f32>, right: Option<f32>) -> Option<f32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(size), None) | (None, Some(size)) => Some(size),
        (None, None) => None,
    }
}

fn merge_max(left: Option<f32>, right: Option<f32>) -> Option<f32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(size), None) | (None, Some(size)) => Some(size),
        (None, None) => None,
    }
}

/// Resolve cyclic percentages in intrinsic sizing inputs for a replaced box.
fn resolve_cyclic_size(value: Dimension, basis: Option<f32>, min_content: bool) -> Option<f32> {
    if min_content && basis.is_none() {
        return match value.expand() {
            ExpandedDimension::Percent(_) => Some(0.0),
            ExpandedDimension::Calc(handle) if handle.has_percentage() => Some(handle.resolve(0.0)),
            _ => value.maybe_resolve(basis),
        };
    }
    value.maybe_resolve(basis)
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
            ExpandedLengthPercentageAuto::Calc(handle) if handle.has_percentage() => {
                Some(handle.resolve(0.0))
            }
            _ => value.maybe_resolve(basis),
        };
    }
    value.maybe_resolve(basis)
}

/// A side of a box resolved against the box `style` sizes: the side itself for
/// `content-box`, and the side less the padding and border for `border-box`.
fn content_side(style: &Style, side: f32, edges: f32) -> f32 {
    match style.box_sizing {
        BoxSizing::ContentBox => side,
        BoxSizing::BorderBox => (side - edges).max(0.0),
    }
}

/// The padding and border of `style` on each axis, with percentages resolved
/// against `basis`, the width of the containing block.
pub(super) fn padding_and_border(style: &Style, basis: Option<f32>) -> Size<f32> {
    (style.padding.resolve_or_zero(basis) + style.border.resolve_or_zero(basis)).sum_axes()
}

/// The part of the `edges` (padding and border) that lies between the content
/// box and the box the preferred aspect ratio relates: all of it for a
/// `border-box` box whose ratio is not a content box one, none otherwise.
pub(super) fn ratio_box_insets(
    style: &Style,
    edges: Size<f32>,
    content_box_ratio: bool,
) -> Size<f32> {
    match style.box_sizing {
        BoxSizing::BorderBox if !content_box_ratio => edges,
        _ => Size::ZERO,
    }
}

/// The content-box height that goes with a content-box `width` along `ratio`,
/// where the ratio relates the boxes that are `insets` larger than the content
/// boxes.
pub(super) fn height_for_width(width: f32, ratio: f32, insets: Size<f32>) -> f32 {
    ((width + insets.width) / ratio - insets.height).max(0.0)
}

/// The content-box width that goes with a content-box `height` along `ratio`;
/// see [`height_for_width`].
pub(super) fn width_for_height(height: f32, ratio: f32, insets: Size<f32>) -> f32 {
    ((height + insets.height) * ratio - insets.width).max(0.0)
}

/// Convert a border-box size to the content-box size returned by this module.
fn content_box_from_border_box(side: f32, edges: f32) -> f32 {
    (side - edges).max(0.0)
}

#[cfg(test)]
mod tests;
