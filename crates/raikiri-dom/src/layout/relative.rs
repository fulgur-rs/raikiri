//! Used offsets of relatively positioned boxes whose offset the painter
//! applies instead of layout.

use super::ifc::style::inset_precedence;
use super::multicol::spanning::is_spanner;
use super::page::{find_body, used_computed_length_percentage_or_auto};
use super::page_pipeline::{box_parent, definite_content_height};
use crate::document::Document;
use crate::fragment::FragmentationContext;
use raikiri_style::property::{DisplayValue, PositionValue, WritingMode};
use raikiri_style::{ComputedLengthPercentageOrAuto, ComputedValues};

/// The used horizontal and vertical insets of `node_id`, whatever its
/// `position`. When opposing insets both apply, the one on the start side
/// of the containing block's writing mode and direction wins (CSS Position
/// 3 §3.3). `None` stands for `auto`.
///
/// Percentages and `calc()` resolve against the content box of the
/// containing block: the nearest ancestor box that is not a non-atomic
/// inline, and inside a multi-column container the column box of a child
/// that does not span the columns. The used size of a flex or grid item is
/// definite. The body is laid out in the page area,
/// so that is its containing block. The width is always known; a
/// percentage of the height counts as `auto` unless that height is definite
/// (CSS 2.1 §9.3.2, §10.5), as in layout.
pub(crate) fn used_relative_insets(
    document: &Document,
    cv: &ComputedValues,
    node_id: usize,
) -> (Option<f32>, Option<f32>) {
    let block = containing_block(document, node_id);
    let rtl =
        block.is_some_and(|block| document.nodes[block].style.direction == taffy::Direction::Rtl);
    let mode = match crate::taffy_impl::used_writing_mode(document, block.unwrap_or(node_id)) {
        WritingMode::VerticalRl => shodo::geometry::WritingMode::VerticalRl,
        WritingMode::VerticalLr => shodo::geometry::WritingMode::VerticalLr,
        WritingMode::SidewaysRl => shodo::geometry::WritingMode::SidewaysRl,
        WritingMode::SidewaysLr => shodo::geometry::WritingMode::SidewaysLr,
        _ => shodo::geometry::WritingMode::HorizontalTb,
    };
    let (right_wins, bottom_wins) = inset_precedence(mode, rtl);
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
    // The tag check spares the document walk for every other element.
    let is_body = || {
        document.nodes[node_id].tag_name() == Some("body") && find_body(document) == Some(node_id)
    };
    let width = || {
        if is_body() {
            return Some(document.fixed_page_area.0);
        }
        let block = block?;
        Some(if is_spanner(document, node_id) {
            content_width(document, block)
        } else {
            column_content_width(document, block)
        })
    };
    let height = || {
        if is_body() {
            return Some(document.fixed_page_area.1);
        }
        let block = block?;
        if is_flex_or_grid_item(document, block) {
            return Some(content_height(document, block));
        }
        definite_content_height(document, block)
    };
    let pick = |start: Option<f32>, end: Option<f32>, end_wins: bool| {
        if end_wins {
            end.or(start)
        } else {
            start.or(end)
        }
    };
    let left = resolve(cv.left, &width);
    let right = resolve(cv.right, &width).map(|right| -right);
    let top = resolve(cv.top, &height);
    let bottom = resolve(cv.bottom, &height).map(|bottom| -bottom);
    (
        pick(left, right, right_wins),
        pick(top, bottom, bottom_wins),
    )
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

/// Whether `block` is laid out as a flex or grid item. Its used size is
/// definite once the container has sized it, auto height or not (CSS Flexbox
/// 1 §9.8, CSS Grid 1 §6.1), so layout resolves percentages inside it
/// against that size.
fn is_flex_or_grid_item(document: &Document, block: usize) -> bool {
    box_parent(document, block).is_some_and(|parent| {
        matches!(
            document.nodes[parent].style.display,
            taffy::Display::Flex | taffy::Display::Grid
        )
    })
}

/// The content-box height of `block`.
fn content_height(document: &Document, block: usize) -> f32 {
    let layout = &document.nodes[block].unrounded_layout;
    layout.size.height
        - layout.padding.top
        - layout.padding.bottom
        - layout.border.top
        - layout.border.bottom
}

/// The content-box width of `block`.
fn content_width(document: &Document, block: usize) -> f32 {
    let layout = &document.nodes[block].unrounded_layout;
    layout.size.width
        - layout.padding.left
        - layout.padding.right
        - layout.border.left
        - layout.border.right
}

/// The content-box width of `block`, or of one of its columns when it is a
/// multi-column container.
fn column_content_width(document: &Document, block: usize) -> f32 {
    let width = content_width(document, block);
    document.nodes[block]
        .multicol
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
