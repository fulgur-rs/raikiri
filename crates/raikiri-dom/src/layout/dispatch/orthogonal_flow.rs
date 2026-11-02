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
//! definite height, else the page area's, cut at the `max-height` of the
//! parent. A vertical box with a width of its own, a float, or an item of a row
//! flex container is not stretched by the parent, yet fits the same height
//! ([`with_page_room`]). A vertical item of a horizontal grid that is asked for
//! the width it gives a column fits the content height of the grid, or no height
//! when the grid has none.

use crate::layout::context::LayoutSession;
use crate::layout::ifc::boxes::{box_sizing_height_adjustment, max_height_cap, open_height_room};
use crate::layout::protocol::{
    AvailableSpace, LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode,
};
use crate::layout::taffy_adapter::columns_follow_their_items;
use crate::node::FormattingContext;
use raikiri_layout_geometry::Size;
use raikiri_layout_style::values::{
    AlignItemsKeyword, BoxSizing, Float, MaybeResolve, Position, ResolveOrZero,
};

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
/// else the height of the initial containing block, the page area here, within
/// the `max-height` of the parent, less its own vertical margins. Whatever
/// else the parent passes down, an intrinsic or a definite space, is kept as it
/// is. The padding and border of the box come off the room where it is laid
/// out.
///
/// A block container laid out on transposed axes takes its margins off the
/// room itself, so it is offered the room whole.
fn offered_height(tree: &LayoutSession<'_>, idx: usize, inputs: &LayoutInput) -> AvailableSpace {
    if inputs.available_space.height != AvailableSpace::MaxContent {
        return inputs.available_space.height;
    }
    let definite = inputs
        .parent_size
        .height
        .filter(|&height| !is_stretch_to_dropped_limit(tree, idx, height));
    page_room(tree, idx, inputs, definite)
}

/// Whether `height`, the definite content height of the parent of `idx`, is the
/// one a `max-height` clamps the parent to as a stretched flex item, a limit
/// that `max_height_cap` drops: the box is then no definite room for `idx`, and
/// it fits the page area, as it does in a parent that is not stretched. A flex
/// container with a height of its own makes the stretch definite, whatever the
/// limit comes to.
fn is_stretch_to_dropped_limit(tree: &LayoutSession<'_>, idx: usize, height: f32) -> bool {
    stretch_to_dropped_limit(tree, idx, height).unwrap_or(false)
}

/// [`is_stretch_to_dropped_limit`], `None` when the parent has no length for a
/// `max-height` or is no flex item of a container with an open height.
fn stretch_to_dropped_limit(tree: &LayoutSession<'_>, idx: usize, height: f32) -> Option<bool> {
    let parent = tree.taffy_parent_of(idx)?;
    let style = tree.physical_style(parent);
    let max = style.max_size().height.maybe_resolve(None::<f32>)?;
    let min = style
        .min_size()
        .height
        .maybe_resolve(None::<f32>)
        .unwrap_or(0.0);
    // The height of a flex container that states its own is definite room for
    // the item it stretches, and the limit is not what settles it.
    let basis = super::flex_item_percent_basis(tree, parent)
        .filter(|basis| basis.height.is_none())?
        .width;
    let padding = style.padding().resolve_or_zero(basis);
    let border = style.border().resolve_or_zero(basis);
    let edges = padding.top + padding.bottom + border.top + border.bottom;
    Some(
        style.box_sizing() == BoxSizing::BorderBox
            && style.size().height.is_auto()
            && max_height_cap(tree, parent).is_none()
            && (height + edges - max.max(min)).abs() < 0.01,
    )
}

/// The room of the margin box of the orthogonal flow `idx` along its inline
/// axis: `definite` when the parent has a definite height, else the height of
/// the page area within the `max-height` of the parent (see `max_height_cap`),
/// less the vertical margins of the box where it does not take them off
/// itself.
fn page_room(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: &LayoutInput,
    definite: Option<f32>,
) -> AvailableSpace {
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
    // Without a page area, or a limit of the container, there is no room to
    // fit the lines in.
    definite
        .or_else(|| {
            let cap = tree
                .taffy_parent_of(idx)
                .and_then(|parent| max_height_cap(tree, parent));
            open_height_room(tree, cap)
        })
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

/// `inputs` for the vertical box `idx` with the height it fits its lines in as
/// the available height, when it is a vertical box in a horizontal flow that is
/// offered none (CSS Writing Modes 4 §7.3.1).
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
        inputs
    } else {
        with_grid_item_room(tree, idx, inputs)
    }
}

/// `inputs` for the vertical item `idx` of a horizontal grid with the height it
/// fits its lines in as the available height, when they ask for the width it
/// gives a column ([`grid_item_room`]). A paragraph root takes it through
/// [`with_page_room`]; a grid container that runs on the axes of its vertical
/// writing mode takes it here.
pub(super) fn with_grid_item_room(
    tree: &LayoutSession<'_>,
    idx: usize,
    mut inputs: LayoutInput,
) -> LayoutInput {
    if let Some(room) = grid_item_room(tree, idx, &inputs) {
        inputs.available_space.height = room;
    }
    inputs
}

/// The horizontal grid container that the in-flow vertical box `idx` is an item
/// of.
fn horizontal_grid_of_vertical_item(tree: &LayoutSession<'_>, idx: usize) -> Option<usize> {
    use super::vertical_oof::used_writing_mode;
    if tree.nodes[idx].layout_position() == Position::Absolute
        || !used_writing_mode(tree, idx).is_vertical()
    {
        return None;
    }
    let grid = tree.taffy_parent_of(idx)?;
    (tree.nodes[grid].formatting_context == FormattingContext::Grid
        && !used_writing_mode(tree, grid).is_vertical())
    .then_some(grid)
}

/// Whether the horizontal grid container `grid` has a vertical item in flow.
pub(super) fn has_vertical_item(tree: &LayoutSession<'_>, grid: NodeId) -> bool {
    tree.child_ids(grid).any(|child| {
        horizontal_grid_of_vertical_item(tree, usize::from(child)) == Some(usize::from(grid))
    })
}

/// Whether the box `idx` fits its lines in the height it is offered, rather than
/// in the height its parent stretches it to: a vertical box in a horizontal
/// block container, a row flex container or a grid container, in flow.
pub(super) fn fits_lines_in_offered_height(tree: &LayoutSession<'_>, idx: usize) -> bool {
    is_vertical_box_in_horizontal_flow(tree, idx)
        || horizontal_grid_of_vertical_item(tree, idx).is_some()
}

/// Whether the vertical item `idx` of a horizontal grid is stretched to the
/// height of its row: its alignment in the block axis of the grid is `normal` or
/// `stretch`, and neither of its vertical margins is `auto` (CSS Box Alignment 3
/// §6.2).
pub(super) fn stretches_to_grid_row(tree: &LayoutSession<'_>, idx: usize) -> bool {
    let Some(grid) = horizontal_grid_of_vertical_item(tree, idx) else {
        return false;
    };
    let style = tree.physical_style(idx);
    let margin = style.margin();
    if margin.top.is_auto() || margin.bottom.is_auto() {
        return false;
    }
    let alignment = style
        .bridge_align_self()
        .unwrap_or_else(|| tree.physical_style(grid).align_items());
    matches!(
        alignment.keyword,
        AlignItemsKeyword::Stretch | AlignItemsKeyword::Normal
    )
}

/// The border-box `height` of the box `idx` within its `min-height` and
/// `max-height`, where they are lengths. `width_basis` is what its percentage
/// padding refers to.
pub(super) fn clamp_to_height_limits(
    tree: &LayoutSession<'_>,
    idx: usize,
    width_basis: Option<f32>,
    height: f32,
) -> f32 {
    let style = tree.physical_style(idx);
    let adjustment = box_sizing_height_adjustment(tree, idx, width_basis.unwrap_or(0.0));
    let max = style
        .max_size()
        .height
        .maybe_resolve(None::<f32>)
        .map(|max| max + adjustment);
    let min = style
        .min_size()
        .height
        .maybe_resolve(None::<f32>)
        .map(|min| min + adjustment);
    let height = max.map_or(height, |max| height.min(max));
    min.map_or(height, |min| height.max(min))
}

/// The height that the vertical items of the grid container `grid` fit their
/// lines in while the grid sizes its columns, as the height of its content box.
///
/// The grid hands its items the height it has: the one it is given or states.
/// A grid that is asked for its min-content or max-content width has a height
/// it is given only where its parent stretches it to a height that is definite,
/// as that of a flex container is when it states its own; otherwise it has the
/// height it states, and a stretch to the lines of its row, which the width is
/// sized before, is no part of its width. A grid whose height is open, but
/// whose `max-height` limits it, hands its items what the limit leaves when it
/// is laid out at a width: its rows grow to the lines of the items up to that,
/// and the items share it by row. That holds where some column follows its
/// items; columns that all have a size of their own leave the rows to the lines
/// of the items, as without a limit. `size` is the border-box size the grid is
/// given or states, `stated` the height it states, and `edges` the padding and
/// border of the grid above and below.
pub(super) fn grid_item_content_height(
    tree: &LayoutSession<'_>,
    grid: usize,
    inputs: &LayoutInput,
    size: Size<Option<f32>>,
    stated: Option<f32>,
    edges: f32,
) -> Option<f32> {
    let style = tree.physical_style(grid);
    let basis = inputs.parent_size;
    let at_a_width = size.width.is_some();
    let height = if at_a_width || basis.height.is_some() {
        size.height
    } else {
        stated
    };
    if let Some(height) = height {
        return Some((height - edges).max(0.0));
    }
    // The limit informs the width of a column that follows its items. A column
    // of a size of its own is laid out with the rows the items make, whatever
    // the limit is.
    if !at_a_width || !columns_follow_their_items(&style) {
        return None;
    }
    let max = style.max_size().height.maybe_resolve(basis.height)?;
    let min = style
        .min_size()
        .height
        .maybe_resolve(basis.height)
        .unwrap_or(0.0);
    let limit = max.max(min);
    Some(if style.box_sizing() == BoxSizing::ContentBox {
        limit
    } else {
        (limit - edges).max(0.0)
    })
}

/// The height that the vertical item `idx` of a horizontal grid fits its lines
/// in, when `inputs` ask for the width it contributes to a column.
///
/// Taffy sizes the columns before the rows, and offers the item a `min-content`
/// or `max-content` height meanwhile, which would break its lines at the
/// smallest height. The room to fit them in is the content height of the grid
/// when that is definite, as for an item of a row flex container, and no limit
/// when it is not: the rows grow to the lines of the item. A request for the
/// height is the very inline size that is measured, and keeps its intrinsic
/// space, as does a layout in rows the grid has settled.
fn grid_item_room(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: &LayoutInput,
) -> Option<AvailableSpace> {
    if inputs.axis != RequestedAxis::Horizontal
        || inputs.run_mode != RunMode::ComputeSize
        || inputs.known_dimensions.height.is_some()
    {
        return None;
    }
    let grid = horizontal_grid_of_vertical_item(tree, idx)?;
    let &(_, height) = tree
        .context
        .grid_inner_heights
        .iter()
        .rev()
        .find(|&&(owner, _)| owner == grid)?;
    Some(match height {
        Some(height) => page_room(tree, idx, inputs, Some(height)),
        None => AvailableSpace::MaxContent,
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
