//! Paint-order and paint-eligibility predicates shared by the paint walker and
//! the page paint-order event generator.

use crate::Document;
use raikiri_style::property::{
    DisplayValue, FloatValue, OverflowValue, PositionValue, Visibility, ZIndexValue,
};
use raikiri_style::{CascadeResult, ComputedValues};
use raikiri_traits::NodeKind;

/// Whether `node_id` is painted on the page whose name is `active_page_name`.
///
/// `None` skips the check. An inline `<canvas>` and floated boxes stay on
/// the page layout placed them on even when their `page` value names
/// another page.
pub fn named_page_matches(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    active_page_name: Option<Option<&str>>,
) -> bool {
    match active_page_name {
        None => true,
        Some(active) => {
            // An inline canvas is a boundary marker for pagination, but its
            // `page` value does not assign the replaced inline box to the
            // named page. Keep painting it on the page selected by layout.
            let is_inline_canvas = document.get_node(node_id).is_some_and(|node| {
                node.tag_name()
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("canvas"))
                    && matches!(cascade.computed[node_id].display, DisplayValue::Inline)
            });
            if is_inline_canvas {
                true
            } else {
                match cascade.page_values.get(node_id) {
                    Some(raikiri_style::property::PageValue::Named(name)) => {
                        // Floats retain the preceding page in the page-name-float
                        // cases; do not hide the floated box merely because its
                        // inherited page value names the following page.
                        if matches!(
                            cascade.computed[node_id].float,
                            FloatValue::Left
                                | FloatValue::Right
                                | FloatValue::InlineStart
                                | FloatValue::InlineEnd
                                | FloatValue::Footnote
                        ) {
                            true
                        } else {
                            active.is_some_and(|page| name.0.as_str() == page)
                        }
                    }
                    _ => true,
                }
            }
        }
    }
}

/// Whether the element clips its descendants to its padding box.
pub fn clips_overflow(cv: &ComputedValues) -> bool {
    !matches!(cv.overflow.x, OverflowValue::Visible)
        || !matches!(cv.overflow.y, OverflowValue::Visible)
}

/// The group opacity the element paints its subtree with, if any.
pub fn opacity_layer(cv: &ComputedValues) -> Option<f32> {
    (cv.opacity < 1.0).then_some(cv.opacity)
}

/// A `visibility: hidden` table does not paint its own box.
pub fn is_visibility_hidden_table(cv: &ComputedValues) -> bool {
    cv.visibility == Visibility::Hidden
        && matches!(cv.display, DisplayValue::Table | DisplayValue::InlineTable)
}

/// Stable-sorts a parent's children into paint order.
pub fn sort_paint_children(
    children: &mut [usize],
    parent_display: DisplayValue,
    cascade: &CascadeResult,
) {
    let order_sensitive_container = matches!(
        parent_display,
        DisplayValue::Flex
            | DisplayValue::InlineFlex
            | DisplayValue::Grid
            | DisplayValue::InlineGrid
    );
    // Stable tuple sorting keeps DOM order for equal (stack, order) values.
    // Direct abspos/fixed children are not flex/grid items: paint them as if
    // their order were zero, while in-flow (including relative) items use the
    // computed `order` value. Stacking buckets stay primary.
    children.sort_by_key(|&child| {
        let computed = &cascade.computed[child];
        let stack =
            if order_sensitive_container && matches!(computed.position, PositionValue::Static) {
                // A flex/grid item can use integer z-index even when static.
                // Match positioned items' stack levels so order only breaks
                // ties within the same z-index level.
                match computed.z_index {
                    ZIndexValue::Auto => (1, 0),
                    ZIndexValue::Integer(value) if value < 0 => (0, value),
                    ZIndexValue::Integer(value) => (2, value),
                    _ => paint_order_key(cascade, child), // cov:ignore: defensive fallback for future non-exhaustive z-index variants.
                }
            } else {
                paint_order_key(cascade, child)
            };
        let order = if order_sensitive_container
            && !matches!(
                computed.position,
                PositionValue::Absolute | PositionValue::Fixed
            ) {
            computed.order
        } else {
            0
        };
        let float_paint_order = if !order_sensitive_container
            && matches!(
                computed.float,
                FloatValue::Left
                    | FloatValue::Right
                    | FloatValue::InlineStart
                    | FloatValue::InlineEnd
            ) {
            1
        } else {
            0
        };
        (stack, float_paint_order, order)
    });
}

fn paint_order_key(cascade: &CascadeResult, node_id: usize) -> (u8, i32) {
    let computed = &cascade.computed[node_id];
    match (&computed.position, computed.z_index) {
        // In-flow boxes paint before positioned descendants with an auto
        // z-index. Keep ordinary static boxes in the normal bucket, but
        // place flex containers with positioned descendants alongside
        // auto-z siblings so their later absolute child paints in tree order.
        (PositionValue::Static, _)
            if matches!(
                computed.display,
                DisplayValue::Flex | DisplayValue::InlineFlex
            ) =>
        {
            (2, 0)
        }
        (PositionValue::Static, _) => (1, 0),
        (_, ZIndexValue::Auto) => (2, 0),
        (_, ZIndexValue::Integer(value)) if value < 0 => (0, value),
        (_, ZIndexValue::Integer(value)) => (2, value),
        // PositionValue and ZIndexValue are non-exhaustive.  New variants
        // retain the default/source-order bucket until stacking support grows.
        _ => (1, 0),
    }
}

/// Walk the Document arena in DFS order; return the first `<body>` element index.
///
/// Use an iterative `Vec` stack (as in cascade §deep_nesting) to avoid
/// stack overflow on deep DOMs. Return `None` for fragments (no `<body>`).
///
/// Skip subtrees with `!is_in_document()` (such as `<template>` descendants)
/// so a hypothetical `<body>` in an inert subtree is not selected. The
/// paint-side and layout-side body lookups are separate to avoid crossing
/// module boundaries, but share the same contract.
pub fn find_paint_root(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root_index()];
    while let Some(id) = stack.pop() {
        let node = doc.get_node(id)?;
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("body") {
            return Some(id);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None
}

#[cfg(test)]
mod tests;
