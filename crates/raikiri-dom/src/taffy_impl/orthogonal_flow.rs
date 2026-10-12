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
    AvailableSpace, BlockContext, Display, LayoutInput, LayoutOutput, NodeId,
    Position as TaffyPosition, Size, compute_flexbox_layout, compute_grid_layout,
};

use super::{block_layout, compute_ifc_root};
use crate::document::Document;
use crate::node::NodeFlags;
use raikiri_style::property::{DisplayValue, WritingMode};

fn is_vertical(mode: WritingMode) -> bool {
    matches!(mode, WritingMode::VerticalRl | WritingMode::VerticalLr)
}

/// Whether `idx` is an in-flow block-level box with an auto width that
/// authors a vertical writing mode under a horizontal block container.
pub(super) fn is_orthogonal_auto_block_size(tree: &Document, idx: usize) -> bool {
    let node = &tree.nodes[idx];
    if !node.authored_writing_mode.is_some_and(is_vertical)
        || !node.style.size.width.is_auto()
        || node.style.position == TaffyPosition::Absolute
        || node.style.float != taffy::Float::None
        || !matches!(
            node.display,
            DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::Flex | DisplayValue::Grid
        )
    {
        return false;
    }
    let Some(parent) = tree.taffy_parent_of(idx) else {
        return false;
    };
    matches!(
        tree.nodes[parent].style.display,
        Display::Block | Display::FlowRoot
    ) && !is_vertical(super::vertical_oof::used_writing_mode(tree, parent))
}

/// Lay out `node_id` with its content-based width in place of the width
/// its block container stretched it to.
pub(super) fn compute_orthogonal_block(
    tree: &mut Document,
    node_id: NodeId,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let ifc_root = tree.nodes[idx].flags.contains(NodeFlags::IS_IFC_ROOT);
    let display = tree.nodes[idx].style.display;
    let run = |tree: &mut Document, inputs: LayoutInput, block_ctx| match display {
        Display::Flex => compute_flexbox_layout(tree, node_id, inputs),
        Display::Grid => compute_grid_layout(tree, node_id, inputs),
        _ if ifc_root => compute_ifc_root(tree, node_id, inputs, block_ctx),
        _ => block_layout(tree, node_id, inputs, block_ctx),
    };
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
    let width = run(tree, measure_inputs, None).size.width;
    let final_inputs = LayoutInput {
        known_dimensions: Size {
            width: Some(width),
            height: inputs.known_dimensions.height,
        },
        available_space: Size {
            width: AvailableSpace::Definite(width),
            height: inputs.available_space.height,
        },
        ..inputs
    };
    run(tree, final_inputs, block_ctx)
}

#[cfg(test)]
mod tests;
