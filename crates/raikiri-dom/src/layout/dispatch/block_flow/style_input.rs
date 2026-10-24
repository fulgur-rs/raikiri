//! Capture the fields project block primitives read through real style queries.

use crate::layout::{context::LayoutSession, style_views::ReadRole};
use raikiri_layout_style::values::{BoxGenerationMode, Display};
use raikiri_layout_style::{BoxStyle, LayoutStyleView};

pub(crate) fn container(tree: &LayoutSession<'_>, idx: usize) -> BoxStyle {
    materialize(tree.query_style(idx, ReadRole::BlockContainer))
}
pub(crate) fn child(tree: &LayoutSession<'_>, idx: usize) -> BoxStyle {
    materialize(tree.query_style(idx, ReadRole::BlockItem))
}

/// The box's own style with no role context, reduced to the fields that
/// block primitives and leaf sizing read, for callers that adjust a local copy
/// before sizing a leaf.
pub(crate) fn physical(tree: &LayoutSession<'_>, idx: usize) -> BoxStyle {
    tree.reduced_physical_style(idx, materialize)
}

fn materialize(query: LayoutStyleView<'_>) -> BoxStyle {
    // Primitives distinguish only hidden, block and non-block display here.
    let display = if query.box_generation_mode() == BoxGenerationMode::None {
        Display::None
    } else if query.is_block() {
        Display::Block
    } else {
        Display::Flex
    };
    BoxStyle {
        display,
        item_is_table: query.is_table(),
        item_is_replaced: query.is_replaced(),
        item_is_compressible_replaced: query.is_compressible_replaced(),
        box_sizing: query.box_sizing(),
        direction: query.direction(),
        overflow: query.overflow(),
        contain: query.contain(),
        position: query.position(),
        inset: query.inset(),
        size: query.size(),
        min_size: query.min_size(),
        max_size: query.max_size(),
        aspect_ratio: query.aspect_ratio(),
        margin: query.margin(),
        padding: query.padding(),
        border: query.border(),
        float: query.float(),
        clear: query.clear(),
        justify_items: query.justify_items(),
        align_content: query.bridge_align_content(),
        align_self: query.bridge_align_self(),
        justify_self: query.bridge_justify_self(),
    }
}
