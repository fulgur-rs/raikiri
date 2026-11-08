//! The first baseline of a grid container.
//!
//! A grid container takes its first baseline from an item in its first row
//! (CSS Grid 1 §11.5). Normalize the native result using the selected item's
//! exported first baseline, grid order, and relative positioning offset.
//! Final layout and isolated row measurement use the same correction.
// spec: https://www.w3.org/TR/css-grid-1/#grid-baselines

use super::axis_adapter::synthesized_baseline;
use super::vertical_block::block_right_to_left;
use crate::document::Document;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutOutput, RunMode};
use crate::layout::style_views::ReadRole;
use crate::layout::taffy_adapter::{GridInfo, RowSizing};
use crate::node::Node;
use raikiri_layout_geometry::{BoxLayout, Size};
use raikiri_layout_style::LayoutStyleView;
use raikiri_layout_style::values::{AlignItemsKeyword, Position};
use std::collections::HashSet;

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

/// Item placement and baseline membership from one native grid report.
pub(crate) struct RecordedItems {
    pub(crate) rows: Box<[(usize, u16)]>,
    pub(crate) row_areas: Box<[crate::node::GridItemRow]>,
    pub(crate) columns: Box<[(usize, u16)]>,
    pub(crate) excluded: Box<[usize]>,
    pub(crate) areas: Box<[(usize, Size<f32>)]>,
}

struct ItemView<'a> {
    rows: &'a [(usize, u16)],
    columns: &'a [(usize, u16)],
    excluded: &'a [usize],
    areas: &'a [(usize, Size<f32>)],
}

impl<'a> ItemView<'a> {
    fn of(node: &'a Node) -> Self {
        Self {
            rows: &node.grid_item_row_starts,
            columns: &node.grid_item_column_starts,
            excluded: &node.grid_items_without_baseline,
            areas: &node.grid_item_areas,
        }
    }
}

/// Read item placement and percentage-dependent baseline exclusions without
/// publishing the report into the document's pagination metadata.
pub(crate) fn recorded_items(
    tree: &LayoutSession<'_>,
    parent: usize,
    info: &GridInfo,
) -> RecordedItems {
    let children = tree
        .child_ids(parent.into())
        .map(usize::from)
        .filter(|&child| {
            tree.nodes[child].layout_display() != raikiri_layout_style::values::Display::None
                && tree.nodes[child].layout_position() != Position::Absolute
        })
        .collect::<Vec<_>>();
    debug_assert_eq!(children.len(), info.items().len());
    let rows = RowSizing::of(&tree.physical_style(parent), info.row_counts());
    let vertical = super::grid_oof::right_to_left(tree, parent).is_some();
    RecordedItems {
        row_areas: children
            .iter()
            .zip(info.items())
            .enumerate()
            .filter_map(|(index, (&item, lines))| {
                let (block_start, _) = info.item_block_range(index)?;
                Some(crate::node::GridItemRow {
                    item,
                    start: lines.row_start,
                    end: lines.row_end,
                    block_start,
                })
            })
            .collect(),
        rows: children
            .iter()
            .zip(info.items())
            .map(|(&child, item)| (child, item.row_start))
            .collect(),
        columns: children
            .iter()
            .zip(info.items())
            .map(|(&child, item)| (child, item.column_start))
            .collect(),
        excluded: children
            .iter()
            .zip(info.items())
            .filter(|&(&child, item)| {
                let span = usize::from(item.row_start.saturating_sub(1))
                    ..usize::from(item.row_end.saturating_sub(1));
                let size = tree.physical_style(child).size();
                (if vertical { size.width } else { size.height }).uses_percentage()
                    && rows.any_sized_by_content_or_free_space(span)
            })
            .map(|(&child, _)| child)
            .collect(),
        areas: children
            .iter()
            .enumerate()
            .filter_map(|(index, &child)| {
                info.item_area_size(index)
                    .map(|area| (child, if vertical { area.transpose() } else { area }))
            })
            .collect(),
    }
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
    first_baseline_in(&ItemView::of(node), is_item, aligns_by_baseline)
}

fn first_baseline_in(
    items: &ItemView<'_>,
    is_item: impl Fn(usize) -> bool,
    aligns_by_baseline: impl Fn(usize) -> bool,
) -> Option<usize> {
    let first_row = items
        .rows
        .iter()
        .filter(|&&(item, _)| is_item(item))
        .map(|&(_, row)| row)
        .min()?;
    // The columns are recorded together with the rows, for the same children
    // in the same order, which follows the children Taffy laid out: in
    // order-modified document order, with `display: contents` flattened.
    debug_assert_eq!(items.rows.len(), items.columns.len());
    let mut first_row_items = items
        .rows
        .iter()
        .zip(items.columns.iter())
        .filter(|&(&(item, row), _)| row == first_row && is_item(item))
        .map(|(&(item, _), &(_, column))| (column, item))
        .collect::<Vec<_>>();
    // A stable sort keeps the order-modified document order within a column.
    first_row_items.sort_by_key(|&(column, _)| column);
    // Build membership once, rather than scanning every exclusion for each
    // baseline candidate. The empty exclusion list needs no bucket storage.
    let excluded: HashSet<_> = items.excluded.iter().copied().collect();
    #[cfg(test)]
    tests::record_insertions(items.excluded.len());
    first_row_items
        .iter()
        .map(|&(_, item)| item)
        .find(|&child| {
            aligns_by_baseline(child) && {
                #[cfg(test)]
                tests::record_query();
                !excluded.contains(&child)
            }
        })
        .or_else(|| first_row_items.first().map(|&(_, item)| item))
}

#[cfg(test)]
mod tests;

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
/// An item with layout containment hands over none (CSS Contain 2 §3.2).
pub(super) fn with_first_baseline(
    tree: &LayoutSession<'_>,
    idx: usize,
    vertical: bool,
    run_mode: RunMode,
    output: LayoutOutput,
) -> LayoutOutput {
    if run_mode != RunMode::PerformLayout {
        return output;
    }
    correct_baseline(
        tree,
        idx,
        vertical,
        output,
        ItemView::of(&tree.nodes[idx]),
        |item| {
            Some((
                tree.nodes[item].layout,
                tree.nodes[item].exported_first_baseline,
            ))
        },
    )
}

/// Correct a measured horizontal grid's baseline from locally captured
/// placement and child results, using the same selection as final layout.
pub(crate) fn with_measured_first_baseline(
    tree: &LayoutSession<'_>,
    idx: usize,
    output: LayoutOutput,
    info: &GridInfo,
    child: impl Fn(usize) -> Option<(BoxLayout, Option<f32>)>,
) -> LayoutOutput {
    let items = recorded_items(tree, idx, info);
    let view = ItemView {
        rows: &items.rows,
        columns: &items.columns,
        excluded: &items.excluded,
        areas: &items.areas,
    };
    correct_baseline(tree, idx, false, output, view, child)
}

fn correct_baseline(
    tree: &LayoutSession<'_>,
    idx: usize,
    vertical: bool,
    mut output: LayoutOutput,
    items: ItemView<'_>,
    child: impl Fn(usize) -> Option<(BoxLayout, Option<f32>)>,
) -> LayoutOutput {
    // Preserve intentional suppression by containment and empty containers.
    if output.baselines.first.is_none() {
        return output;
    }
    let container = tree.physical_style(idx);
    let aligns = |item| aligns_by_baseline(&container, &tree.physical_style(item), vertical);
    let item = first_baseline_in(&items, |_| true, aligns)
        .filter(|&item| !tree.nodes[item].opposed_baseline_group);
    if let Some(item) = item
        && let Some((layout, exported)) = child(item)
    {
        let height = layout.size.height;
        let baseline = exported.unwrap_or_else(|| {
            if vertical {
                synthesized_baseline(tree, idx, height)
            } else {
                height
            }
        });
        let basis = items
            .areas
            .iter()
            .find(|&&(held, _)| held == item)
            .map(|&(_, area)| if vertical { area.width } else { area.height });
        let top = layout.location.y - relative_block_offset(tree, item, basis);
        output.baselines.first = Some(top + baseline);
    }
    output
}

/// How far the algorithm of the grid container `idx` moved its item `item`
/// along the block axis for being positioned relatively: `top` wins over
/// `bottom` on the axes the algorithm runs on, and a percentage refers to the
/// grid area of the item.
fn relative_block_offset(tree: &LayoutSession<'_>, item: usize, basis: Option<f32>) -> f32 {
    // A relative offset that layout leaves to the painter is not in the
    // location.
    if tree.physical_style(item).position() != Position::Relative
        || tree.nodes[item].needs_relative_block_paint_offset
    {
        return 0.0;
    }
    let inset = tree.query_style(item, ReadRole::GridItem).inset();
    let (start, end) = (inset.top.resolve(basis), inset.bottom.resolve(basis));
    start.or(end.map(|end| -end)).unwrap_or(0.0)
}

/// Replace the first baseline in `output`, the result of the vertical grid
/// container `idx` whose block flow runs from right to left when
/// `right_to_left`, with the one of the item that gives it its baseline when
/// that item runs against the block direction of the container and takes part
/// in baseline alignment.
///
/// Such an item is aligned by the container only once the algorithm is done,
/// so [`with_first_baseline`] leaves it out; its border box is where that
/// alignment moved it, and its baseline is the one it was aligned by. The baseline of the container is an
/// offset from its block-start edge, the right one when `right_to_left`.
pub(super) fn with_opposed_first_baseline(
    tree: &LayoutSession<'_>,
    idx: usize,
    right_to_left: bool,
    mut output: LayoutOutput,
) -> LayoutOutput {
    if output.baselines.first.is_none() {
        return output;
    }
    let view = ItemView::of(&tree.nodes[idx]);
    // A grid with no opposed item keeps the baseline the algorithm found.
    if !view
        .rows
        .iter()
        .any(|&(item, _)| tree.nodes[item].opposed_baseline_group)
    {
        return output;
    }
    let container = tree.physical_style(idx);
    // The bridge leaves an opposed item at `start` for the algorithm, so it
    // does not align by a baseline there.
    let aligns = |item: usize| {
        tree.nodes[item].opposed_baseline_group
            || aligns_by_baseline(&container, &tree.physical_style(item), true)
    };
    let Some(item) = first_baseline_in(&view, |_| true, aligns)
        .filter(|&item| tree.nodes[item].opposed_baseline_group)
    else {
        return output;
    };
    let layout = tree.nodes[item].layout;
    let from_left = super::inline_baseline::opposed_baseline_from_left(tree, idx, item);
    let basis = view
        .areas
        .iter()
        .find(|&&(held, _)| held == item)
        .map(|&(_, area)| area.width);
    let x = layout.location.x - relative_x_offset(tree, item, right_to_left, basis) + from_left;
    output.baselines.first = Some(if right_to_left {
        output.size.width - x
    } else {
        x
    });
    output
}

/// How far being positioned relatively moved the item `item` of a vertical
/// grid container to the right: the inset at the block-start side of the
/// container wins, the right one when `right_to_left`. A percentage refers to
/// `basis`, the width of the item's grid area.
fn relative_x_offset(
    tree: &LayoutSession<'_>,
    item: usize,
    right_to_left: bool,
    basis: Option<f32>,
) -> f32 {
    let style = tree.physical_style(item);
    if style.position() != Position::Relative || tree.nodes[item].needs_relative_block_paint_offset
    {
        return 0.0;
    }
    let inset = style.inset();
    let left = inset.left.resolve(basis);
    let right = inset.right.resolve(basis).map(|right| -right);
    if right_to_left {
        right.or(left)
    } else {
        left.or(right)
    }
    .unwrap_or(0.0)
}
