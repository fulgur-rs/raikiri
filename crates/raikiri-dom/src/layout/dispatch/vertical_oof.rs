//! Absolutely positioned children of containers in vertical writing modes.
//!
//! Block layout runs on physical axes, so a vertical block container with
//! only out-of-flow children ends with a zero physical height. Taffy hands
//! that height to each absolutely positioned child as its available height
//! and places the child at the physical left edge. In vertical writing
//! modes the physical height is the inline axis: a shrink-to-fit inline size
//! then collapses to min-content, and the static position belongs at the
//! block-start edge (CSS Position 3 §4.3, CSS 2.1 §10.3.7, both read through
//! CSS Writing Modes 4 §7.1 abstract-to-physical mapping).
//!
//! This module records the inline extent such a container would fill, gives
//! it to the out-of-flow children as their available inline size, and moves
//! their static position to the block-start edge. Its `alignment` submodule
//! aligns them by the axes of the writing mode.
//!
//! A flex or grid container laid out on the axes of its own writing mode
//! converts the positioning area and the static positions of its
//! out-of-flow children to physical axes when it leaves that frame, so those
//! children are placed like the children of a block container. Their
//! coordinates come from Taffy's positioning of the candidates (`oof`) and
//! from the alignment of `alignment`; the page layout rewrites them only for
//! the auto margins of the boxes that the root contains.

use crate::layout::protocol::{AvailableSpace, LayoutInput};
use raikiri_layout_geometry::Size;
use raikiri_layout_style::values::{LengthExt, MaybeResolve, Position, ResolveOrZero};

use crate::document::Document;
use crate::layout::context::LayoutSession;
use crate::node::NodeFlags;
use raikiri_style::property::WritingMode;

mod alignment;

pub(super) use alignment::{
    align_hoisted_children, align_hoisted_children_in_grid, hoisted_overflow_rect,
};

/// The writing mode resolved against the DOM by the last style bridge.
pub(crate) fn used_writing_mode(tree: &Document, node: usize) -> WritingMode {
    tree.nodes[node].used_writing_mode
}

fn is_absolute(tree: &Document, node: usize) -> bool {
    tree.nodes[node].layout_position() == Position::Absolute && !tree.nodes[node].is_display_none()
}

/// The physical height of the padding box `node` would fill as a vertical
/// block container with an auto inline size, when its out-of-flow children
/// need it as their containing block's inline size.
///
/// `None` when the node is horizontal, has no absolutely positioned child,
/// or already has a definite physical height that Taffy uses as the
/// containing block.
pub(super) fn containing_block_inline_size(
    tree: &LayoutSession<'_>,
    node: usize,
    inputs: &LayoutInput,
) -> Option<f32> {
    if inputs.known_dimensions.height.is_some()
        || !tree.nodes[node]
            .children
            .iter()
            .any(|&child| is_absolute(tree, child))
        || !used_writing_mode(tree, node).is_vertical()
        // A transposed block container hands its children its own used
        // inline size.
        || super::block_right_to_left(tree, node).is_some()
    {
        return None;
    }
    let style = tree.physical_style(node);
    if style
        .size()
        .height
        .maybe_resolve(inputs.parent_size.height)
        .is_some()
    {
        return None;
    }
    // A block-level box fills the inline size of its containing block, which
    // is the parent's physical height in vertical writing modes.
    let available = match (inputs.parent_size.height, inputs.available_space.height) {
        (Some(height), _) | (None, AvailableSpace::Definite(height)) => height,
        (None, _) => return None,
    };
    let basis = Some(available);
    let margin = style
        .margin()
        .map(|margin| margin.resolve(Some(available)).unwrap_or(0.0));
    let border = style.border().resolve_or_zero(basis);
    let mut border_box = (available - margin.top - margin.bottom).max(0.0);
    if let Some(max) = style.max_size().height.maybe_resolve(basis) {
        border_box = border_box.min(max);
    }
    if let Some(min) = style.min_size().height.maybe_resolve(basis) {
        border_box = border_box.max(min);
    }
    Some((border_box - border.top - border.bottom).max(0.0))
}

/// `inputs` for the absolutely positioned `node`, with the inline extent of
/// a vertical containing block recorded by [`containing_block_inline_size`]
/// as its available physical height.
pub(super) fn absolute_child_inputs(
    tree: &LayoutSession<'_>,
    node: usize,
    inputs: LayoutInput,
) -> LayoutInput {
    let Some(&(container, inline_size)) = tree.context.vertical_oof_containing_blocks.last() else {
        return inputs;
    };
    let parent = tree
        .anonymous_cell_of(node)
        .or_else(|| tree.parent_of(node));
    if parent != Some(container)
        || !is_absolute(tree, node)
        || inputs.known_dimensions.height.is_some()
    {
        return inputs;
    }
    let inset = tree.physical_style(node).inset();
    if !inset.top.is_auto() && !inset.bottom.is_auto() {
        return inputs;
    }
    LayoutInput {
        parent_size: Size {
            height: Some(inline_size),
            ..inputs.parent_size
        },
        available_space: Size {
            height: AvailableSpace::Definite(inline_size),
            ..inputs.available_space
        },
        ..inputs
    }
}

/// Whether the subtree of `node` holds inline content laid out by the
/// inline engine. In flow, such a box makes its vertical container lay out
/// its block children on vertical axes; a box without it stays on the
/// horizontal block path.
fn has_inline_content(tree: &Document, node: usize) -> bool {
    let mut stack = vec![node];
    while let Some(id) = stack.pop() {
        if tree.nodes[id].flags.contains(NodeFlags::IS_IFC_ROOT) {
            return true;
        }
        stack.extend(tree.nodes[id].children.iter().copied());
    }
    false
}

/// Move each absolutely positioned child of the vertical container `node`
/// whose `left` and `right` are both auto, and which holds inline content,
/// to its static position on the block-start edge of the padding box: the
/// right edge in `vertical-rl`, the left edge in `vertical-lr`. A container
/// laid out here has no in-flow content before the child, so that edge is
/// the static position. A child without inline content keeps the position
/// the horizontal block path gives the same box in flow.
pub(super) fn place_static_block_start(
    tree: &mut LayoutSession<'_>,
    node: usize,
    border_box: Size<f32>,
) {
    let children = &tree.nodes[node].children;
    let in_flow = |child: usize| {
        tree.nodes[child].kind() == raikiri_traits::NodeKind::Element
            && !is_absolute(tree, child)
            && !tree.nodes[child].is_display_none()
    };
    if !children.iter().any(|&child| is_absolute(tree, child))
        || children.iter().any(|&child| in_flow(child))
    {
        return;
    }
    let mode = used_writing_mode(tree, node);
    if !mode.is_vertical() {
        return;
    }
    let (border, padding) = {
        let style = tree.physical_style(node);
        (
            style.border().resolve_or_zero(Some(border_box.height)),
            style.padding().resolve_or_zero(Some(border_box.height)),
        )
    };
    let children: Vec<usize> = tree.nodes[node]
        .children
        .iter()
        .copied()
        .filter(|&child| is_absolute(tree, child) && has_inline_content(tree, child))
        .collect();
    for child in children {
        let child_style = tree.physical_style(child);
        let inset = child_style.inset();
        if !inset.left.is_auto() || !inset.right.is_auto() {
            continue;
        }
        let margin = child_style
            .margin()
            .map(|margin| margin.resolve(Some(border_box.height)).unwrap_or(0.0));
        let layout = &mut tree.nodes[child].layout;
        layout.location.x = match mode {
            WritingMode::VerticalRl | WritingMode::SidewaysRl => {
                border_box.width - border.right - padding.right - margin.right - layout.size.width
            }
            _ => border.left + padding.left + margin.left,
        };
    }
}

#[cfg(test)]
mod tests;
