//! Column fragmentation of block children that hold no lines.
//!
//! A block box with no in-flow content of its own (for example a sized,
//! empty `div`) has no line boxes to break between, but it is still
//! fragmentable: a box taller than the remaining column continues in the
//! next column (CSS Fragmentation 3, 4.4; CSS Multi-column 1, 7.1). The other
//! strategies place such a box atomically in one column. This one lays the
//! children out as one stream and slices each box at the column boundaries.

use super::*;
use crate::fragment::{FragmentRect, LayoutFragment};
use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};
use raikiri_style::property::BreakInside;

/// The rendered children of `parent` when every one of them is a plain
/// block with no content, and at least one of them has a block size.
///
/// Inline margins, generated content, forced or avoided breaks between
/// boxes, positioned or floated boxes, scroll containers and replaced
/// elements need their own handling at a break, so containers with them keep
/// the other strategies.
pub(super) fn supported(tree: &Document, parent: usize) -> Option<Vec<usize>> {
    let container = &tree.nodes[parent];
    if container.ifc.is_some()
        || container.style.direction != TaffyDirection::Ltr
        || !inherits_horizontal(tree, parent)
        || container.has_before_or_after_content
    {
        return None;
    }
    let mut children = Vec::new();
    for &child in &container.children {
        let node = &tree.nodes[child];
        if !node.is_in_document() || node.is_display_none() {
            continue;
        }
        // Text that renders gives the container lines, refused above, so any
        // other node here is collapsible white space.
        let NodeData::Element(_) = &node.data else {
            continue;
        };
        let defaults: taffy::Style = taffy::Style::default();
        if node.display != DisplayValue::Block
            || node.style.direction != TaffyDirection::Ltr
            // An orthogonal flow is monolithic in the column's block axis.
            || !is_horizontal(node)
            // So is a box with size containment (CSS Contain 2, 3.1).
            || node.size_containment.height
            || node.style.position != TaffyPosition::Relative
                && node.style.position != defaults.position
            || node.style.float.is_floated()
            || node.style.clear != defaults.clear
            || node.style.inset != Rect::auto()
            || block_margins(node).is_none()
            || node.style.overflow != defaults.overflow
            || node.tag_name().is_some_and(|tag| {
                ATOMIC_TAGS.contains(&tag)
                    || REPLACED_BOX_TAGS.contains(&tag)
                    || matches!(tag, "fieldset" | "legend" | "details" | "summary")
            })
            || node.break_before != BreakBetween::Auto
            || node.break_after != BreakBetween::Auto
            || node.has_before_or_after_content
            || node.multicol.is_some()
            || node.ifc.is_some()
            || has_rendered_content(tree, child)
        {
            return None;
        }
        children.push(child);
    }
    children
        .iter()
        .any(|&child| {
            let style = &tree.nodes[child].style;
            style_dimension_length(style.size.height).is_some()
                || style_dimension_length(style.min_size.height.into()).is_some_and(|px| px > 0.0)
        })
        .then_some(children)
}

/// Whether the bridged CSS writing mode of `id` is horizontal.
fn inherits_horizontal(tree: &Document, id: usize) -> bool {
    is_horizontal(&tree.nodes[id])
}

fn is_horizontal(node: &crate::Node) -> bool {
    !node.used_writing_mode.is_vertical()
}

/// The block-start and block-end margins of a child, when they are
/// non-negative lengths and its inline margins are zero.
fn block_margins(node: &crate::Node) -> Option<(f32, f32)> {
    let zero = LengthPercentageAuto::length(0.0);
    let margin = node.style.margin;
    if margin.left != zero || margin.right != zero {
        return None;
    }
    let length = |value: LengthPercentageAuto| {
        style_dimension_length(value.into()).filter(|px| px.is_finite() && *px >= 0.0)
    };
    Some((length(margin.top)?, length(margin.bottom)?))
}

fn has_block_edge_decorations(node: &crate::Node) -> bool {
    let zero = LengthPercentage::length(0.0);
    let (padding, border) = (node.style.padding, node.style.border);
    padding.top != zero || padding.bottom != zero || border.top != zero || border.bottom != zero
}

fn has_rendered_content(tree: &Document, id: usize) -> bool {
    tree.nodes[id].children.iter().any(|&child| {
        let node = &tree.nodes[child];
        node.is_in_document()
            && !node.is_display_none()
            && match &node.data {
                NodeData::Text(text) => !text.text_content.trim().is_empty(),
                _ => true,
            }
    })
}

/// One column's share of a child box.
#[derive(Debug, PartialEq)]
struct Piece {
    column: usize,
    /// Block offset of the piece from the top of its column.
    top: f32,
    height: f32,
}

/// A child box in the stream: its block size, its block margins, whether
/// it may break, and whether cutting it at a column end is refused.
#[derive(Clone, Copy)]
struct Item {
    height: f32,
    margin_top: f32,
    margin_bottom: f32,
    splittable: bool,
    whole: bool,
}

const EPSILON: f32 = 0.001;

/// Why a placement stopped before the end of the stream.
#[derive(Debug, PartialEq)]
enum Stop {
    /// More pieces or columns were needed than allowed.
    Exhausted,
    /// A box that must stay whole reached a column end.
    Cut,
}

/// The pieces of every child, and where the stream ends.
struct Placement {
    pieces: Vec<Vec<Piece>>,
    /// Block offset in the last column after the last child's block-end
    /// margin, which stays inside the multicol container.
    end: f32,
}

/// Fill columns of `column_height` with `items` in order. A splittable box
/// continues at the top of the next column; a box that avoids breaks moves
/// to the next column when it does not fit below earlier content, overflows
/// a column it starts at the top of, and breaks anyway when it is taller than
/// a whole column (CSS Fragmentation 3, 4.4). A box that avoids breaks and
/// must stay whole overflows instead.
/// Adjoining sibling margins collapse, and a margin at a break is truncated
/// (CSS Fragmentation 3, 5.2). Stops with [`Stop::Exhausted`] once more
/// than `max_pieces` pieces or skipped columns would be needed, so a tall box
/// in short columns cannot allocate or loop past the fragment budget, and
/// with [`Stop::Cut`] as soon as a box that must stay whole would be cut.
fn place(items: &[Item], column_height: f32, max_pieces: usize) -> Result<Placement, Stop> {
    place_within(items, column_height, max_pieces, usize::MAX, true)
}

/// [`place`], also stopping once the stream needs more than `max_columns`
/// columns. Boxes that must stay whole are cut like any other unless
/// `keep_whole` is set.
fn place_within(
    items: &[Item],
    column_height: f32,
    max_pieces: usize,
    max_columns: usize,
    keep_whole: bool,
) -> Result<Placement, Stop> {
    place_columns(
        items,
        &|_| column_height,
        max_pieces,
        max_columns,
        keep_whole,
    )
}

/// [`place_within`] with a block size of its own for each column.
fn place_columns(
    items: &[Item],
    height_of: &dyn Fn(usize) -> f32,
    max_pieces: usize,
    max_columns: usize,
    keep_whole: bool,
) -> Result<Placement, Stop> {
    let mut column = 0usize;
    let mut cursor = 0.0_f32;
    let mut pending_margin = 0.0_f32;
    let mut piece_count = 0usize;
    let mut placements = Vec::with_capacity(items.len());
    for item in items {
        // A box with no block size, borders or padding is self-collapsing:
        // its own margins join the margins around it (CSS 2, 8.3.1).
        if item.height <= 0.0 {
            piece_count += 1;
            if piece_count > max_pieces {
                return Err(Stop::Exhausted);
            }
            placements.push(vec![Piece {
                column,
                top: (cursor + pending_margin.max(item.margin_top)).min(height_of(column)),
                height: 0.0,
            }]);
            pending_margin = pending_margin.max(item.margin_top).max(item.margin_bottom);
            continue;
        }
        let mut pieces = Vec::new();
        let mut gap = pending_margin.max(item.margin_top);
        // A box that must stay whole moves to the next column like a box
        // that avoids breaks, rather than being cut, when it fits there: in
        // the rest of this column, or in a whole new one if content precedes
        // it here.
        let splittable = item.splittable
            && !(item.whole
                && (cursor + gap + item.height <= height_of(column) + EPSILON
                    || cursor > 0.0 && item.height <= height_of(column + 1) + EPSILON));
        let fits_after_gap = if splittable {
            cursor + gap < height_of(column) - EPSILON
        } else {
            cursor + gap + item.height <= height_of(column) + EPSILON
        };
        // A gap that fills the column leaves no room for a box that avoids
        // breaks either, so it starts the next column with the gap truncated.
        let gap_fills_column = !splittable && gap >= height_of(column) - EPSILON;
        if (cursor > 0.0 || gap_fills_column)
            && !fits_after_gap
            && (item.height > 0.0 || !splittable)
        {
            column += 1;
            if column >= max_columns {
                return Err(Stop::Exhausted);
            }
            cursor = 0.0;
            gap = 0.0;
        }
        cursor += gap;
        pending_margin = item.margin_bottom;
        // A box that avoids breaks but is taller than a whole column breaks
        // anyway (CSS Fragmentation 3, 4.4), unless painting cannot slice it.
        let breaks_anyway = !splittable && !item.whole && item.height > height_of(column) + EPSILON;
        if splittable || breaks_anyway {
            let mut left = item.height;
            loop {
                let room = height_of(column) - cursor;
                // A gap that fills the column leaves no room for the box,
                // which then starts in the next column.
                if room < -EPSILON || (room <= EPSILON && left > EPSILON) {
                    // Columns too short for any of the box at their top
                    // would cut it at every one of them.
                    if keep_whole && item.whole && cursor <= 0.0 {
                        return Err(Stop::Cut);
                    }
                    piece_count += 1;
                    if piece_count > max_pieces {
                        return Err(Stop::Exhausted);
                    }
                    column += 1;
                    if column >= max_columns {
                        return Err(Stop::Exhausted);
                    }
                    cursor = 0.0;
                    continue;
                }
                piece_count += 1;
                if piece_count > max_pieces {
                    return Err(Stop::Exhausted);
                }
                if left <= room + EPSILON {
                    pieces.push(Piece {
                        column,
                        top: cursor,
                        height: left,
                    });
                    cursor += left;
                    break;
                }
                if keep_whole && item.whole {
                    return Err(Stop::Cut);
                }
                pieces.push(Piece {
                    column,
                    top: cursor,
                    height: room,
                });
                left -= room;
                column += 1;
                if column >= max_columns {
                    return Err(Stop::Exhausted);
                }
                cursor = 0.0;
            }
        } else {
            piece_count += 1;
            if piece_count > max_pieces {
                return Err(Stop::Exhausted);
            }
            pieces.push(Piece {
                column,
                top: cursor,
                height: item.height,
            });
            cursor += item.height;
        }
        placements.push(pieces);
    }
    Ok(Placement {
        pieces: placements,
        end: cursor + pending_margin,
    })
}

impl Placement {
    fn columns_used(&self) -> usize {
        self.pieces
            .iter()
            .flatten()
            .map(|piece| piece.column + 1)
            .max()
            .unwrap_or(0)
    }

    /// The tallest column from column `first` on, counting the trailing
    /// margin of the stream.
    fn extent_after(&self, first: usize) -> f32 {
        self.pieces
            .iter()
            .flatten()
            .filter(|piece| piece.column >= first)
            .map(|piece| piece.top + piece.height)
            .fold(self.end, f32::max)
    }
}

/// The block size of the stream in one column of unlimited height, with
/// adjoining margins collapsed.
fn stacked_height(items: &[Item], max_pieces: usize) -> Option<f32> {
    place(items, f32::INFINITY, max_pieces)
        .ok()
        .map(|placement| placement.end)
}

/// The smallest column height that fits `items` in `column_count` columns.
/// Content that may break anywhere shares the columns equally; boxes that
/// avoid breaks can need taller columns (CSS Multi-column 1, 7.1). The
/// search ignores the fragment budget, which the final placement enforces,
/// so running out of fragments is not mistaken for columns that are too
/// short. A placement needs at least one piece per column, so columns past
/// `max_pieces` already exceed the budget and bound each probe.
fn balanced_height(items: &[Item], column_count: usize, max_pieces: usize) -> f32 {
    let max_columns = column_count.min(max_pieces.saturating_add(1));
    // One unbounded column takes a piece per box, which the unlimited
    // budget always allows.
    let high = stacked_height(items, usize::MAX).unwrap_or(0.0);
    balanced_height_after(items, 0, &|_| 0.0, column_count, max_columns, high, false)
}

/// [`balanced_height`] for the `column_count` columns that follow the first
/// `fixed` columns, whose heights `fixed_height` gives, with the content
/// placed in at most `max_columns` columns after those. Boxes that must stay
/// whole are not cut when `keep_whole` is set. `high` bounds the search
/// from above and is the answer when nothing shorter fits.
fn balanced_height_after(
    items: &[Item],
    fixed: usize,
    fixed_height: &dyn Fn(usize) -> f32,
    column_count: usize,
    max_columns: usize,
    high: f32,
    keep_whole: bool,
) -> f32 {
    let limit = fixed.saturating_add(max_columns);
    let probe = |height: f32| {
        let height_of = |column: usize| {
            if column < fixed {
                fixed_height(column)
            } else {
                height
            }
        };
        place_columns(items, &height_of, usize::MAX, limit, keep_whole).ok()
    };
    // Every column holds its content, and the last one the trailing margin.
    let fits = |height: f32| {
        probe(height).is_some_and(|placement| {
            placement.columns_used() <= limit && placement.extent_after(fixed) <= height + EPSILON
        })
    };
    // Margins collapse and are truncated at breaks, so only the boxes
    // themselves bound the height from below: the content left after the
    // fixed columns, shared equally, and the tallest box that avoids breaks
    // and fits within `high` (a taller one breaks anyway).
    let reference = probe(high);
    let before: f32 = reference.as_ref().map_or(0.0, |placement| {
        placement
            .pieces
            .iter()
            .flatten()
            .filter(|piece| piece.column < fixed)
            .map(|piece| piece.height)
            .sum()
    });
    let content: f32 = items.iter().map(|item| item.height).sum();
    // Boxes the fixed columns already hold do not bound the rest.
    let left = |index: usize| {
        reference.as_ref().is_none_or(|placement| {
            placement.pieces[index]
                .iter()
                .any(|piece| piece.column >= fixed)
        })
    };
    let tallest = items
        .iter()
        .enumerate()
        .filter(|&(index, item)| !item.splittable && item.height <= high + EPSILON && left(index))
        .map(|(_, item)| item.height)
        .fold(0.0_f32, f32::max);
    let mut low = ((content - before) / column_count as f32).max(tallest);
    if low > 0.0 && low <= high && fits(low) {
        return low;
    }
    let mut high = high.max(low);
    // Each probe places the children once; the bound keeps the search linear
    // in the number of children.
    for _ in 0..24 {
        let middle = (low + high) / 2.0;
        if fits(middle) {
            high = middle;
        } else {
            low = middle;
        }
    }
    // The search stops near the answer; the content extent of the columns
    // it found is the exact height when that still fits.
    let extent = probe(high).map_or(0.0, |placement| placement.extent_after(fixed));
    if extent > 0.0 && fits(extent) {
        extent
    } else {
        high
    }
}

/// The stream item of `child`, a box of block size `height`.
fn item(tree: &Document, child: usize, height: f32) -> Item {
    let node = &tree.nodes[child];
    let (margin_top, margin_bottom) = block_margins(node).unwrap_or_default();
    Item {
        height,
        margin_top,
        margin_bottom,
        splittable: !matches!(
            node.break_inside,
            BreakInside::Avoid | BreakInside::AvoidColumn
        ),
        // Painting draws every fragment's decorations whole, so a box with
        // block-axis borders or padding, or with effects sized from the
        // whole box, would repeat them at each break instead of slicing them
        // (CSS Fragmentation 3, 5.4). Such a box may still sit whole in one
        // column.
        whole: has_block_edge_decorations(node) || node.paints_whole_box_effects,
    }
}

/// One piece of a child in a row of a nested container's columns.
#[derive(Debug, PartialEq)]
pub(super) struct RowPiece {
    /// Index of the child in the stream.
    pub(super) entry: usize,
    pub(super) row: usize,
    /// Column within the row; the last row may hold overflow columns.
    pub(super) column: usize,
    /// Block offset of the piece from the row's top.
    pub(super) y: f32,
    pub(super) height: f32,
    /// Index of the piece among the child's pieces, and their count.
    pub(super) index: usize,
    pub(super) count: usize,
}

/// The pieces of a nested container's children in rows of its columns,
/// and the used block size of each row.
#[derive(Debug, PartialEq)]
pub(super) struct Rows {
    pub(super) pieces: Vec<RowPiece>,
    pub(super) heights: Vec<f32>,
}

/// Fill the children (from [`supported`]) of a nested column container,
/// whose own columns are cut again at the end of each outer column, into
/// rows of `column_count` columns: the first row holds `first` of block
/// size and every later row `later` (CSS Fragmentation 3, 2). A container
/// with a definite content height `content` ends with the row that reaches
/// it, which takes the content that is left in overflow columns (CSS
/// Multi-column 1, 8.2); an auto-height one ends with its content, and a
/// balanced one balances its last row. `None` when one row holds everything,
/// when a box that must stay whole would be cut, or past `max_pieces`.
#[allow(clippy::too_many_arguments)]
pub(super) fn fill_rows(
    tree: &Document,
    children: &[usize],
    column_count: usize,
    column_fill: ColumnFillValue,
    content: Option<f32>,
    first: f32,
    later: f32,
    max_pieces: usize,
) -> Option<Rows> {
    let items: Vec<Item> = children
        .iter()
        .map(|&child| item(tree, child, tree.nodes[child].unrounded_layout.size.height))
        .collect();
    fill_item_rows(
        &items,
        column_count,
        column_fill,
        content,
        first,
        later,
        max_pieces,
    )
}

/// [`fill_rows`] for the stream `items`.
fn fill_item_rows(
    items: &[Item],
    column_count: usize,
    column_fill: ColumnFillValue,
    content: Option<f32>,
    first: f32,
    later: f32,
    max_pieces: usize,
) -> Option<Rows> {
    if column_count == 0 || !(first > EPSILON && later > EPSILON) || !later.is_finite() {
        return None;
    }
    let row_of = |column: usize, last: usize| (column / column_count).min(last);
    let (placement, heights) = if let Some(content) = content {
        let mut heights = Vec::new();
        let mut left = content;
        while left > EPSILON {
            let height = left.min(if heights.is_empty() { first } else { later });
            heights.push(height);
            left -= height;
            if heights.len() > max_pieces {
                return None;
            }
        }
        if heights.len() < 2 {
            return None;
        }
        let last = heights.len() - 1;
        let height_of = |column: usize| heights[row_of(column, last)];
        let placement = place_columns(items, &height_of, max_pieces, usize::MAX, true).ok()?;
        // Content that fits the last row's columns is balanced in them
        // unless they fill sequentially; the row keeps its height.
        let fixed = column_count * last;
        let placement = if column_fill != ColumnFillValue::Auto
            && placement.columns_used() <= fixed + column_count
        {
            let height = balanced_height_after(
                items,
                fixed,
                &height_of,
                column_count,
                column_count,
                heights[last],
                true,
            );
            let balanced = |column: usize| {
                if column < fixed {
                    height_of(column)
                } else {
                    height
                }
            };
            place_columns(items, &balanced, max_pieces, usize::MAX, true).ok()?
        } else {
            placement
        };
        (placement, heights)
    } else {
        let height_of = |column: usize| if column < column_count { first } else { later };
        let placement = place_columns(items, &height_of, max_pieces, usize::MAX, true).ok()?;
        let last = (placement.columns_used().max(1) - 1) / column_count;
        if last == 0 {
            return None;
        }
        // The last row is as tall as its content, or balanced: the shortest
        // height that keeps the content in that row's columns.
        let fixed = column_count * last;
        let placement = if column_fill == ColumnFillValue::Auto {
            placement
        } else {
            let height = balanced_height_after(
                items,
                fixed,
                &height_of,
                column_count,
                column_count,
                later,
                true,
            );
            let balanced = |column: usize| {
                if column < fixed {
                    height_of(column)
                } else {
                    height
                }
            };
            place_columns(items, &balanced, max_pieces, fixed + column_count, true).ok()?
        };
        let mut heights: Vec<f32> = (0..last)
            .map(|row| if row == 0 { first } else { later })
            .collect();
        heights.push(placement.extent_after(fixed));
        (placement, heights)
    };
    let last = heights.len() - 1;
    let pieces = placement
        .pieces
        .iter()
        .enumerate()
        .flat_map(|(entry, pieces)| {
            let count = pieces.len();
            pieces.iter().enumerate().map(move |(index, piece)| {
                let row = row_of(piece.column, last);
                RowPiece {
                    entry,
                    row,
                    column: piece.column - row * column_count,
                    y: piece.top,
                    height: piece.height,
                    index,
                    count,
                }
            })
        })
        .collect();
    Some(Rows { pieces, heights })
}

/// Lay out `children` (from [`supported`]) as one stream cut into columns.
/// `minimum` is the authored minimum border-box height of the container,
/// which also bounds its columns from below. `definite` says layout fixed
/// the container's height; otherwise the columns balance, bounded by the
/// content-box `maximum` when there is one. Returns the used
/// border-box height of the container.
#[allow(clippy::too_many_arguments)]
pub(super) fn layout(
    tree: &mut Document,
    parent: usize,
    children: &[usize],
    context: FragmentationContext,
    border_box: Size<f32>,
    origin: Point<f32>,
    block_end_inset: f32,
    minimum: f32,
    definite: bool,
    maximum: Option<f32>,
    outer_height: Option<f32>,
) -> Option<f32> {
    // A definite container uses the content height layout gave it, which
    // already reflects min/max constraints and stretching by its parent.
    let definite_height =
        definite.then(|| (border_box.height - origin.y - block_end_inset).max(0.0));
    let mut sizes = Vec::with_capacity(children.len());
    for &child in children {
        tree.nodes[child].cache.clear();
        let output = tree.compute_child_layout(
            TaffyNodeId::from(child),
            super::column_child_input(
                super::column_child_known_width(tree, child, context.column_width),
                context.column_width,
                definite_height,
                AvailableSpace::MaxContent,
            ),
        );
        sizes.push(output.size);
    }
    let items: Vec<Item> = children
        .iter()
        .zip(&sizes)
        .map(|(&child, size)| item(tree, child, size.height))
        .collect();
    // One fragment for the container, the rest for the child pieces.
    let max_pieces = tree
        .fragment_tree
        .limit
        .saturating_sub(tree.fragment_tree.fragments.len())
        .saturating_sub(1);
    let column_height = match definite_height {
        Some(height) => height,
        None if context.column_fill == ColumnFillValue::Auto => {
            // A single unbounded column cuts nothing, so only the number of
            // boxes can run out of fragments.
            let Some(height) = stacked_height(&items, max_pieces) else {
                tree.fragment_tree.limit_exceeded = true;
                return None;
            };
            height
        }
        None => balanced_height(&items, context.column_count, max_pieces),
    };
    // A maximum height bounds the columns of an auto-height container.
    let column_height = match (definite_height, maximum) {
        (None, Some(maximum)) => column_height.min(maximum),
        _ => column_height,
    };
    // Columns are as tall as the container's content box (CSS Multi-column
    // 1, 2), so a minimum height makes them taller, whether the authored
    // height is definite or not.
    let column_height = column_height.max(minimum - origin.y - block_end_inset);
    if !column_height.is_finite() || column_height <= 0.0 {
        return None;
    }
    // A box that must stay whole falls back to the other strategies whole
    // when it would be cut, which is not a layout failure; running out of
    // fragments before that is.
    let placements = match place(&items, column_height, max_pieces) {
        Ok(placements) => placements,
        Err(Stop::Cut) => return None,
        Err(Stop::Exhausted) => {
            tree.fragment_tree.limit_exceeded = true;
            return None;
        }
    };
    let used_height = if definite_height.is_some() {
        border_box.height
    } else {
        origin.y + column_height + block_end_inset
    };
    // Columns sliced here are not cut again at the end of an outer column,
    // so a nested container taller than the outer column is left to the
    // strategies that are.
    if outer_height.is_some_and(|outer| used_height > outer + EPSILON) {
        return None;
    }
    tree.fragment_tree.retire_previous_roots(parent);
    tree.nodes[parent].multicol_groups.clear();
    tree.nodes[parent].multicol_rows.clear();
    let container = tree.fragment_tree.try_push(LayoutFragment {
        node_id: parent,
        parent: None,
        fragmentainer: context.column_index,
        rect: FragmentRect {
            x: context.origin_x,
            y: context.origin_y,
            width: border_box.width,
            height: used_height,
        },
        fragmentainer_clip: None,
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    })?;
    let mut previous_column = 0usize;
    for (order, (&child, pieces)) in children.iter().zip(&placements.pieces).enumerate() {
        let size = sizes[order];
        let first = pieces.first()?;
        // A break between two children is a resumable point of the flow.
        if first.column > previous_column && first.top == 0.0 {
            tree.fragment_tree
                .record_break(crate::fragment::BreakToken {
                    node_id: parent,
                    child_index: order,
                    line_index: 0,
                });
        }
        previous_column = pieces.last()?.column;
        let mut layout = tree.nodes[child].unrounded_layout;
        layout.order = order as u32;
        layout.size = size;
        layout.location = Point {
            x: origin.x + context.column_offset_x(first.column),
            y: origin.y + first.top,
        };
        tree.set_unrounded_layout(TaffyNodeId::from(child), &layout);
        for (index, piece) in pieces.iter().enumerate() {
            let x = origin.x + context.column_offset_x(piece.column);
            tree.fragment_tree.try_push(LayoutFragment {
                node_id: child,
                parent: Some(container),
                fragmentainer: context.column_index + piece.column,
                rect: FragmentRect {
                    x,
                    y: origin.y + piece.top,
                    width: size.width,
                    height: piece.height,
                },
                // A box that stays in one column is not cut.
                fragmentainer_clip: (pieces.len() > 1).then_some(FragmentRect {
                    x,
                    y: origin.y,
                    width: size.width.max(context.column_width),
                    height: column_height,
                }),
                fragment_index: index,
                fragment_count: pieces.len(),
                line_start: None,
                line_end: None,
            })?;
        }
    }
    Some(used_height)
}

#[cfg(test)]
mod tests;
