//! The first baseline of a grid container.
//!
//! A grid container takes its first baseline from an item in its first row
//! (CSS Grid 1 §11.5). Taffy hands over that item's bottom edge unless the
//! item shares a baseline with another one in its row, so the baseline is
//! replaced here, once the grid algorithm has placed the items, by the one of
//! the first line the item itself holds.
// spec: https://www.w3.org/TR/css-grid-1/#grid-baselines

use super::axis_adapter::synthesized_baseline;
use super::vertical_block::block_right_to_left;
use crate::document::Document;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutOutput, RunMode};
use crate::layout::style_views::ReadRole;
use crate::node::Node;
use raikiri_layout_style::LayoutStyleView;
use raikiri_layout_style::values::{AlignItemsKeyword, Contain, Position};

/// Whether the grid container `grid` runs its algorithm on the axes of a
/// vertical writing mode.
pub(crate) fn runs_vertically(document: &Document, grid: usize) -> bool {
    block_right_to_left(document, grid).is_some()
}

/// Whether the grid item with style `item` takes part in baseline alignment
/// in the block axis of the grid container with style `container`: its
/// `align-self` resolves to a baseline and neither of its block-axis margins
/// is `auto` (CSS Box Alignment 3 §9.3). `vertical` tells whether the
/// container runs on the axes of a vertical writing mode, whose block-axis
/// margins are the left and right ones.
///
/// A `last baseline` counts too, as the algorithm aligns it like a first one.
pub(crate) fn aligns_by_baseline(
    container: &LayoutStyleView<'_>,
    item: &LayoutStyleView<'_>,
    vertical: bool,
) -> bool {
    let align = item.bridge_align_self().or(Some(container.align_items()));
    let margin = item.margin();
    let auto_block_margin = if vertical {
        margin.left.is_auto() || margin.right.is_auto()
    } else {
        margin.top.is_auto() || margin.bottom.is_auto()
    };
    align.is_some_and(|align| align.keyword == AlignItemsKeyword::Baseline) && !auto_block_margin
}

/// The item of the grid container `node` that gives it its first baseline.
///
/// Among the items in the first row that holds any, in grid order (by column,
/// then in order-modified document order), the first one that
/// `aligns_by_baseline` and takes part in baseline alignment gives the line,
/// or else the first one. An item whose percentage height depends on the rows
/// it spans takes no part (CSS Grid 1 §6.4). `is_item` tells which of the
/// recorded children are items of the container; the children the layout
/// of the container recorded are all of them.
pub(crate) fn first_baseline_item(
    node: &Node,
    is_item: impl Fn(usize) -> bool,
    aligns_by_baseline: impl Fn(usize) -> bool,
) -> Option<usize> {
    let first_row = node
        .grid_item_row_starts
        .iter()
        .filter(|&&(item, _)| is_item(item))
        .map(|&(_, row)| row)
        .min()?;
    // The columns are recorded together with the rows, for the same children
    // in the same order, which follows the children Taffy laid out: in
    // order-modified document order, with `display: contents` flattened.
    debug_assert_eq!(
        node.grid_item_row_starts.len(),
        node.grid_item_column_starts.len()
    );
    let mut first_row_items = node
        .grid_item_row_starts
        .iter()
        .zip(node.grid_item_column_starts.iter())
        .filter(|&(&(item, row), _)| row == first_row && is_item(item))
        .map(|(&(item, _), &(_, column))| (column, item))
        .collect::<Vec<_>>();
    // A stable sort keeps the order-modified document order within a column.
    first_row_items.sort_by_key(|&(column, _)| column);
    first_row_items
        .iter()
        .map(|&(_, item)| item)
        .find(|&child| {
            aligns_by_baseline(child) && !node.grid_items_without_baseline.contains(&child)
        })
        .or_else(|| first_row_items.first().map(|&(_, item)| item))
}

/// Replace the first baseline in `output`, the result of the grid algorithm
/// for the container `idx`, with the one of the first line of the item that
/// gives the container its baseline. `vertical` tells whether the container
/// runs its algorithm on the axes of a vertical writing mode.
///
/// The item's border box lies where the algorithm put it, less the offset of
/// a relatively positioned box, which moves the box and not its baseline. The
/// baseline is the one the item handed over when the algorithm last laid it
/// out: an offset from its block-start border edge, or, for an item that has
/// none, one synthesized from its border box (CSS Box Alignment 3 §9.1).
/// Layout containment suppresses the baseline of the item (CSS Contain 2
/// §3.2).
pub(super) fn with_first_baseline(
    tree: &LayoutSession<'_>,
    idx: usize,
    vertical: bool,
    run_mode: RunMode,
    mut output: LayoutOutput,
) -> LayoutOutput {
    // Layout containment, or a container without items, leaves it without.
    if run_mode != RunMode::PerformLayout || output.baselines.first.is_none() {
        return output;
    }
    let container = tree.physical_style(idx);
    let aligns =
        |child: usize| aligns_by_baseline(&container, &tree.physical_style(child), vertical);
    // The container moves an item that forms a baseline-sharing group of its
    // own after the algorithm has run, which the baseline has not seen: the
    // algorithm's stays.
    let item = first_baseline_item(&tree.nodes[idx], |_| true, aligns)
        .filter(|&item| !tree.nodes[item].opposed_baseline_group);
    if let Some(item) = item {
        let node = &tree.nodes[item];
        let contained = tree
            .physical_style(item)
            .contain()
            .contains(Contain::LAYOUT);
        let height = node.layout.size.height;
        let baseline = node
            .exported_first_baseline
            .filter(|_| !contained)
            .unwrap_or_else(|| {
                // The line-under edge of a box on horizontal axes is its
                // bottom, whatever its writing mode.
                if vertical {
                    synthesized_baseline(tree, idx, height)
                } else {
                    height
                }
            });
        let top = node.layout.location.y - relative_block_offset(tree, idx, item, vertical);
        output.baselines.first = Some(top + baseline);
    }
    output
}

/// How far the algorithm of the grid container `idx` moved its item `item`
/// along the block axis for being positioned relatively: `top` wins over
/// `bottom` on the axes the algorithm runs on, and a percentage refers to the
/// grid area of the item.
fn relative_block_offset(tree: &LayoutSession<'_>, idx: usize, item: usize, vertical: bool) -> f32 {
    // A relative offset that layout leaves to the painter is not in the
    // location.
    if tree.physical_style(item).position() != Position::Relative
        || tree.nodes[item].needs_relative_block_paint_offset
    {
        return 0.0;
    }
    // The areas are on physical axes.
    let basis = tree.nodes[idx]
        .grid_item_areas
        .iter()
        .find(|&&(child, _)| child == item)
        .map(|&(_, area)| if vertical { area.width } else { area.height });
    let inset = tree.query_style(item, ReadRole::GridItem).inset();
    let (start, end) = (inset.top.resolve(basis), inset.bottom.resolve(basis));
    start.or(end.map(|end| -end)).unwrap_or(0.0)
}
