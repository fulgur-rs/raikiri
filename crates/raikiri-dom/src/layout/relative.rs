//! Used offsets of relatively positioned boxes whose offset the painter
//! applies instead of layout.

use super::page::used_computed_length_percentage_or_auto;
use super::page_pipeline::{box_parent, definite_content_height};
use crate::document::Document;
use crate::fragment::FragmentationContext;
use raikiri_style::property::{DisplayValue, PositionValue};
use raikiri_style::{ComputedLengthPercentageOrAuto, ComputedValues};

/// The used horizontal and vertical insets of `node_id`, whatever its
/// `position`. `top` wins over `-bottom`, and `left` over `-right` unless
/// the containing block is right-to-left (CSS 2.1 §9.4.3). `None` stands
/// for `auto`.
///
/// Percentages and `calc()` resolve against the content box of the
/// containing block: the nearest ancestor box that is not a non-atomic
/// inline, and inside a multi-column container its column box. Its width is
/// always known; a percentage of its height counts as `auto` unless that
/// height is definite (CSS 2.1 §9.3.2, §10.5), as in layout.
pub(crate) fn used_relative_insets(
    document: &Document,
    cv: &ComputedValues,
    node_id: usize,
) -> (Option<f32>, Option<f32>) {
    let block = containing_block(document, node_id);
    let rtl =
        block.is_some_and(|block| document.nodes[block].style.direction == taffy::Direction::Rtl);
    let length = |value| match value {
        ComputedLengthPercentageOrAuto::Px(px) => Some(Some(px)),
        ComputedLengthPercentageOrAuto::Auto | ComputedLengthPercentageOrAuto::MinContent => {
            Some(None)
        }
        ComputedLengthPercentageOrAuto::Percent(_) | ComputedLengthPercentageOrAuto::Calc(_) => {
            None
        }
    };
    // The bases are only looked up for an inset that needs them.
    let resolve = |value, basis: &dyn Fn() -> Option<f32>| {
        length(value).unwrap_or_else(|| {
            basis().and_then(|basis| used_computed_length_percentage_or_auto(value, basis))
        })
    };
    let width = || block.map(|block| column_content_width(document, block));
    let height = || block.and_then(|block| definite_content_height(document, block));
    let (start, end) = if rtl {
        (cv.right, cv.left)
    } else {
        (cv.left, cv.right)
    };
    let sign = if rtl { -1.0 } else { 1.0 };
    let dx = resolve(start, &width)
        .map(|start| sign * start)
        .or_else(|| resolve(end, &width).map(|end| -sign * end));
    let dy = resolve(cv.top, &height).or_else(|| resolve(cv.bottom, &height).map(|b| -b));
    (dx, dy)
}

/// The nearest ancestor of `node_id` that can be its containing block:
/// `display: contents` elements and non-atomic inlines are skipped.
fn containing_block(document: &Document, node_id: usize) -> Option<usize> {
    let mut block = box_parent(document, node_id)?;
    while document.nodes[block].display == DisplayValue::Inline {
        block = box_parent(document, block)?;
    }
    Some(block)
}

/// The content-box width of `block`, or of one of its columns when it is a
/// multi-column container.
fn column_content_width(document: &Document, block: usize) -> f32 {
    let node = &document.nodes[block];
    let layout = &node.unrounded_layout;
    let width = layout.size.width
        - layout.padding.left
        - layout.padding.right
        - layout.border.left
        - layout.border.right;
    node.multicol
        .and_then(|style| FragmentationContext::resolve(width, None, style))
        .map_or(width, |context| context.column_width)
}

/// The offset by which a `position: relative` box at `node_id` moves from
/// its layout location, as the painter applies it to boxes that layout
/// places without it; `(0, 0)` for any other `position`.
pub fn used_relative_offset(
    document: &Document,
    cv: &ComputedValues,
    node_id: usize,
) -> (f32, f32) {
    if cv.position != PositionValue::Relative {
        return (0.0, 0.0);
    }
    let (dx, dy) = used_relative_insets(document, cv, node_id);
    (dx.unwrap_or(0.0), dy.unwrap_or(0.0))
}

#[cfg(test)]
mod tests;
