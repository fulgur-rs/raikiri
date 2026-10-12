//! Block-level boxes in a vertical writing mode whose parent is a horizontal
//! block container.
//!
//! Such a box is an orthogonal flow (CSS Writing Modes 4 §7.3): its block
//! axis is the parent's inline axis. Taffy's block algorithm stretches every
//! auto-width child to the container's width, which here is the child's
//! block size. An auto block size is the extent of the box's content
//! instead (CSS 2.1 §10.6.3, read through the abstract-to-physical mapping
//! of CSS Writing Modes 4 §7.1). A stretched vertical box puts its content
//! at the block-start edge, which is the far right in `vertical-rl`, away
//! from where a browser draws it.
//!
//! This module measures such a box at its max-content width, the extent of
//! its columns, and lays it out at that width. Its inline size keeps
//! following the available height the parent passes down.

use taffy::tree::{RequestedAxis, RunMode};
use taffy::{
    AvailableSpace, Display, LayoutInput, LayoutOutput, NodeId, Position as TaffyPosition, Size,
};

use crate::document::Document;
use raikiri_style::property::{DisplayValue, WritingMode};

fn is_vertical(mode: WritingMode) -> bool {
    matches!(mode, WritingMode::VerticalRl | WritingMode::VerticalLr)
}

/// Whether `idx` is an in-flow block-level box with an auto width in a
/// vertical writing mode, its own or inherited, under a horizontal block
/// container.
pub(super) fn is_orthogonal_auto_block_size(tree: &Document, idx: usize) -> bool {
    use super::vertical_oof::used_writing_mode;
    let node = &tree.nodes[idx];
    node.style.size.width.is_auto()
        && node.style.position != TaffyPosition::Absolute
        && node.style.float == taffy::Float::None
        && matches!(
            node.display,
            DisplayValue::Block
                | DisplayValue::FlowRoot
                | DisplayValue::ListItem
                | DisplayValue::Flex
                | DisplayValue::Grid
        )
        && is_vertical(used_writing_mode(tree, idx))
        && tree.taffy_parent_of(idx).is_some_and(|parent| {
            matches!(
                tree.nodes[parent].style.display,
                Display::Block | Display::FlowRoot
            ) && !is_vertical(used_writing_mode(tree, parent))
        })
}

/// Lay out `node_id` with its content-based width in place of the width
/// its block container stretched it to.
///
/// Both passes go through the ordinary dispatch, so replaced leaves,
/// multicol containers and the out-of-flow handling of vertical block
/// containers keep their own paths. The final pass marks the node so that
/// dispatch does not divert it here again.
///
/// An orthogonal flow establishes an independent formatting context (CSS
/// Writing Modes 4 §7.3), so neither pass continues the parent's floats.
pub(super) fn compute_orthogonal_block(
    tree: &mut Document,
    node_id: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let measure_inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Horizontal,
        known_dimensions: Size {
            width: None,
            height: inputs.known_dimensions.height,
        },
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height: inputs.available_space.height,
        },
        ..inputs
    };
    let width = tree
        .compute_child_layout_with_block_ctx(node_id, measure_inputs, None)
        .size
        .width;
    // The width comes from the box's own content, so its percentage-sized
    // children do not resolve against it.
    let final_inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(width),
            height: inputs.known_dimensions.height,
        },
        known_dimensions_are_definite: Size {
            width: false,
            ..inputs.known_dimensions_are_definite
        },
        available_space: Size {
            width: AvailableSpace::Definite(width),
            height: inputs.available_space.height,
        },
        ..inputs
    };
    let outer = tree.orthogonal_block_in_layout.replace(idx);
    let output = tree.compute_child_layout_with_block_ctx(node_id, final_inputs, None);
    tree.orthogonal_block_in_layout = outer;
    output
}

#[cfg(test)]
mod tests;
