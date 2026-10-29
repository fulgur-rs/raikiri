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
//! its columns, and lays it out at that width. Its inline size fits the
//! height it is offered: the one the parent passes down, else the parent's
//! definite height, else the page area's. A vertical paragraph root with a
//! width of its own, a float, or an item of a row flex container is not
//! stretched by the parent, yet fits the same height ([`with_page_room`]).

use crate::layout::context::LayoutSession;
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode,
};
use crate::node::FormattingContext;
use raikiri_layout_geometry::Size;
use raikiri_layout_style::values::{Float, Position, ResolveOrZero};

use raikiri_style::property::DisplayValue;

/// Whether `idx` is an in-flow block-level box with an auto width in a
/// vertical writing mode, its own or inherited, under a horizontal block
/// container.
pub(super) fn is_orthogonal_auto_block_size(tree: &LayoutSession<'_>, idx: usize) -> bool {
    use super::vertical_oof::used_writing_mode;
    let node = &tree.nodes[idx];
    // The style is read last: most boxes are settled by the cheaper tests.
    node.layout_position() != Position::Absolute
        && (node.display().is_block_flex_or_grid_container()
            || matches!(
                node.display(),
                DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
            ))
        && used_writing_mode(tree, idx).is_vertical()
        && tree.taffy_parent_of(idx).is_some_and(|parent| {
            !matches!(
                tree.nodes[parent].formatting_context,
                FormattingContext::None | FormattingContext::Flex | FormattingContext::Grid
            ) && !used_writing_mode(tree, parent).is_vertical()
        })
        && {
            let style = tree.physical_style(idx);
            style.size().width.is_auto() && style.float() == Float::None
        }
}

/// The height that the orthogonal flow `idx` is offered to fit its lines in
/// (CSS Writing Modes 4 §7.3.1), as the room of its margin box.
///
/// A block container leaves the height it passes down to its children
/// indefinite. The box is then offered the definite height of its parent, or
/// else the height of the initial containing block, the page area here, less
/// its own vertical margins. Whatever else the parent passes down, an
/// intrinsic or a definite space, is kept as it is. The padding and border of
/// the box come off the room where it is laid out.
///
/// A block container laid out on transposed axes takes its margins off the
/// room itself, so it is offered the room whole.
fn offered_height(tree: &LayoutSession<'_>, idx: usize, inputs: &LayoutInput) -> AvailableSpace {
    if inputs.available_space.height != AvailableSpace::MaxContent {
        return inputs.available_space.height;
    }
    let node = &tree.nodes[idx];
    let takes_margins_off_itself =
        !node.is_ifc_root() && !node.display().is_flex_or_grid_container();
    // The percentages of the margins refer to the width of the parent, which
    // a flex container has not passed down while it measures its items.
    let basis = inputs.parent_size.width.or_else(|| {
        super::flex_item_percent_basis(tree, idx).and_then(|container| container.width)
    });
    let margin = tree
        .physical_style(idx)
        .margin()
        .map(|margin| margin.resolve_or_zero(basis));
    let margin = if takes_margins_off_itself {
        0.0
    } else {
        margin.top + margin.bottom
    };
    // Without a page area there is no room to fit the lines in.
    inputs
        .parent_size
        .height
        .or_else(|| crate::layout::ifc::boxes::page_area_height(tree))
        .map_or(AvailableSpace::MaxContent, |room| {
            AvailableSpace::Definite((room - margin).max(0.0))
        })
}

/// Whether `idx` is a vertical box that is a child of a horizontal block
/// container, floated or not, or an item of a horizontal flex container with a
/// row direction. The block axis of such a box is the inline axis of its
/// parent, and its lines fit the height its parent leaves open.
///
/// A column flex container sizes the box along its inline axis by its
/// max-content size, and a grid container by its tracks, so neither is one.
fn is_vertical_box_in_horizontal_flow(tree: &LayoutSession<'_>, idx: usize) -> bool {
    use super::vertical_oof::used_writing_mode;
    tree.nodes[idx].layout_position() != Position::Absolute
        && used_writing_mode(tree, idx).is_vertical()
        && tree.taffy_parent_of(idx).is_some_and(|parent| {
            !used_writing_mode(tree, parent).is_vertical()
                && match tree.nodes[parent].formatting_context {
                    FormattingContext::Block => true,
                    FormattingContext::Flex => tree.physical_style(parent).flex_direction_is_row(),
                    _ => false,
                }
        })
}

/// `inputs` for the paragraph root `idx` with the height it fits its lines in
/// as the available height, when it is a vertical box in a horizontal flow
/// that is offered none (CSS Writing Modes 4 §7.3.1).
///
/// The lines of such a box break at its inline size, its height. A parent that
/// leaves that height indefinite, as a block container and a row flex container
/// whose height is automatic do, would have them stay on one column; see
/// [`offered_height`] for the room they get instead.
pub(super) fn with_page_room(
    tree: &LayoutSession<'_>,
    idx: usize,
    mut inputs: LayoutInput,
) -> LayoutInput {
    if is_vertical_box_in_horizontal_flow(tree, idx) {
        inputs.available_space.height = offered_height(tree, idx, &inputs);
    }
    inputs
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
    tree: &mut LayoutSession<'_>,
    node_id: NodeId,
    inputs: LayoutInput,
) -> LayoutOutput {
    let idx = usize::from(node_id);
    let height = offered_height(tree, idx, &inputs);
    let measure_inputs = LayoutInput {
        run_mode: RunMode::ComputeSize,
        axis: RequestedAxis::Horizontal,
        known_dimensions: Size {
            width: None,
            height: inputs.known_dimensions.height,
        },
        available_space: Size {
            width: AvailableSpace::MaxContent,
            height,
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
            height,
        },
        ..inputs
    };
    let outer = tree.context.orthogonal_block_in_layout.replace(idx);
    let output = tree.compute_child_layout_with_block_ctx(node_id, final_inputs, None);
    tree.context.orthogonal_block_in_layout = outer;
    output
}

#[cfg(test)]
mod tests;
