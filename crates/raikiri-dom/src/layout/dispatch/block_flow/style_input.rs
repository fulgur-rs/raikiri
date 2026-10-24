//! Capture the fields project block primitives read through real style queries.

use crate::{
    LayoutStyle,
    layout::{context::LayoutSession, style_views::ReadRole},
};
use raikiri_layout_style::LayoutStyleView;
use taffy::{BlockContainerStyle as BC, BlockItemStyle as BI, CoreStyle};

pub(crate) fn container(tree: &LayoutSession<'_>, idx: usize) -> LayoutStyle {
    materialize(tree.query_style(idx, ReadRole::BlockContainer))
}
pub(crate) fn child(tree: &LayoutSession<'_>, idx: usize) -> LayoutStyle {
    materialize(tree.query_style(idx, ReadRole::BlockItem))
}

/// The box's own style with no role context, reduced to the fields that
/// block primitives and leaf sizing read, for callers that adjust a local copy
/// before sizing a leaf. Flex and grid container and item fields keep their
/// defaults.
pub(crate) fn physical(tree: &LayoutSession<'_>, idx: usize) -> LayoutStyle {
    tree.reduced_physical_style(idx, materialize)
}

fn materialize(query: LayoutStyleView<'_>) -> LayoutStyle {
    // Primitives distinguish only hidden, block and non-block display here.
    let display = if query.box_generation_mode() == taffy::BoxGenerationMode::None {
        taffy::Display::None
    } else if query.is_block() {
        taffy::Display::Block
    } else {
        taffy::Display::Flex
    };
    LayoutStyle {
        display,
        item_is_table: BI::is_table(&query),
        item_is_replaced: query.is_replaced(),
        item_is_compressible_replaced: query.is_compressible_replaced(),
        box_sizing: query.box_sizing(),
        direction: query.direction(),
        overflow: query.overflow(),
        scrollbar_width: query.scrollbar_width(),
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
        float: BI::float(&query),
        clear: BI::clear(&query),
        text_align: BC::text_align(&query),
        justify_items: BC::justify_items(&query),
        align_content: query.bridge_align_content(),
        align_self: query.bridge_align_self(),
        justify_self: query.bridge_justify_self(),
        ..LayoutStyle::default()
    }
}
