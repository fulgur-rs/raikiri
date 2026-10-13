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
        if !node.is_in_document() || node.style.display == Display::None {
            continue;
        }
        match &node.data {
            NodeData::Text(text) if text.text_content.trim().is_empty() => continue,
            NodeData::Element(_) => {}
            _ => return None,
        }
        let defaults: taffy::Style = taffy::Style::default();
        if node.display != DisplayValue::Block
            || node.style.direction != TaffyDirection::Ltr
            // An orthogonal flow is monolithic in the column's block axis.
            || !is_horizontal(node)
            || node.style.position != TaffyPosition::Relative
                && node.style.position != defaults.position
            || node.style.float.is_floated()
            || node.style.clear != defaults.clear
            || node.style.inset != Rect::auto()
            || block_margins(node).is_none()
            || node.style.overflow != defaults.overflow
            // A logical minimum block size keeps the boxes on the projection
            // that preserves their unfragmented minimum.
            || node.has_logical_min_block_size
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

/// Whether the used writing mode of `id`, declared on it or inherited from
/// the nearest ancestor that declares one, is horizontal.
fn inherits_horizontal(tree: &Document, id: usize) -> bool {
    let mut current = Some(id);
    while let Some(id) = current {
        if tree.nodes[id].authored_writing_mode.is_some() {
            return is_horizontal(&tree.nodes[id]);
        }
        current = tree.parent_of(id);
    }
    true
}

fn is_horizontal(node: &crate::Node) -> bool {
    node.authored_writing_mode
        .is_none_or(|mode| mode == raikiri_style::property::WritingMode::HorizontalTb)
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
            && node.style.display != Display::None
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

/// A child box in the stream: its block size, its block margins, and
/// whether it may break.
#[derive(Clone, Copy)]
struct Item {
    height: f32,
    margin_top: f32,
    margin_bottom: f32,
    splittable: bool,
}

const EPSILON: f32 = 0.001;

/// The pieces of every child, and where the stream ends.
struct Placement {
    pieces: Vec<Vec<Piece>>,
    /// Block offset in the last column after the last child's block-end
    /// margin, which stays inside the multicol container.
    end: f32,
}

/// Fill columns of `column_height` with `items` in order. A splittable box
/// continues at the top of the next column; a box that avoids breaks moves
/// to the next column when it does not fit below earlier content, and
/// overflows a column it starts at the top of (CSS Fragmentation 3, 4.4).
/// Adjoining sibling margins collapse, and a margin at a break is truncated
/// (CSS Fragmentation 3, 5.2). Returns `None` once more than `max_pieces`
/// pieces or skipped columns would be needed, so a tall box in short columns
/// cannot allocate or loop past the fragment budget.
fn place(items: &[Item], column_height: f32, max_pieces: usize) -> Option<Placement> {
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
                return None;
            }
            placements.push(vec![Piece {
                column,
                top: (cursor + pending_margin.max(item.margin_top)).min(column_height),
                height: 0.0,
            }]);
            pending_margin = pending_margin.max(item.margin_top).max(item.margin_bottom);
            continue;
        }
        let mut pieces = Vec::new();
        let mut gap = pending_margin.max(item.margin_top);
        let fits_after_gap = if item.splittable {
            cursor + gap < column_height - EPSILON
        } else {
            cursor + gap + item.height <= column_height + EPSILON
        };
        // A gap that fills the column leaves no room for a box that avoids
        // breaks either, so it starts the next column with the gap truncated.
        let gap_fills_column = !item.splittable && gap >= column_height - EPSILON;
        if (cursor > 0.0 || gap_fills_column)
            && !fits_after_gap
            && (item.height > 0.0 || !item.splittable)
        {
            column += 1;
            cursor = 0.0;
            gap = 0.0;
        }
        cursor += gap;
        pending_margin = item.margin_bottom;
        if item.splittable {
            let mut left = item.height;
            loop {
                let room = column_height - cursor;
                // A gap that fills the column leaves no room for the box,
                // which then starts in the next column.
                if room < -EPSILON || (room <= EPSILON && left > EPSILON) {
                    piece_count += 1;
                    if piece_count > max_pieces {
                        return None;
                    }
                    column += 1;
                    cursor = 0.0;
                    continue;
                }
                piece_count += 1;
                if piece_count > max_pieces {
                    return None;
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
                pieces.push(Piece {
                    column,
                    top: cursor,
                    height: room,
                });
                left -= room;
                column += 1;
                cursor = 0.0;
            }
        } else {
            piece_count += 1;
            if piece_count > max_pieces {
                return None;
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
    Some(Placement {
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

    /// The tallest column, counting the trailing margin of the stream.
    fn extent(&self) -> f32 {
        self.pieces
            .iter()
            .flatten()
            .map(|piece| piece.top + piece.height)
            .fold(self.end, f32::max)
    }
}

/// The block size of the stream in one column of unlimited height, with
/// adjoining margins collapsed.
fn stacked_height(items: &[Item], max_pieces: usize) -> Option<f32> {
    place(items, f32::INFINITY, max_pieces).map(|placement| placement.end)
}

/// The smallest column height that fits `items` in `column_count` columns.
/// Content that may break anywhere shares the columns equally; boxes that
/// avoid breaks can need taller columns (CSS Multi-column 1, 7.1).
fn balanced_height(items: &[Item], column_count: usize, max_pieces: usize) -> f32 {
    // Every column holds its content, and the last one the trailing margin.
    let fits = |height: f32| {
        place(items, height, max_pieces).is_some_and(|placement| {
            placement.columns_used() <= column_count && placement.extent() <= height + EPSILON
        })
    };
    // Margins collapse and are truncated at breaks, so only the boxes
    // themselves bound the height from below.
    let content: f32 = items.iter().map(|item| item.height).sum();
    let tallest = items
        .iter()
        .filter(|item| !item.splittable)
        .map(|item| item.height)
        .fold(0.0_f32, f32::max);
    let mut low = (content / column_count as f32).max(tallest);
    if low > 0.0 && fits(low) {
        return low;
    }
    let Some(mut high) = stacked_height(items, max_pieces) else {
        return low;
    };
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
    let extent = place(items, high, max_pieces).map_or(0.0, |placement| placement.extent());
    if extent > 0.0 && fits(extent) {
        extent
    } else {
        high
    }
}

/// Lay out `children` (from [`supported`]) as one stream cut into columns.
/// `minimum` is the authored minimum border-box height of the container,
/// which also bounds its columns from below. Returns the used
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
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Both,
                known_dimensions: Size {
                    width: column_child_known_width(tree, child, context.column_width),
                    height: None,
                },
                known_dimensions_are_definite: Size {
                    width: true,
                    height: false,
                },
                parent_size: Size {
                    width: Some(context.column_width),
                    height: definite_height,
                },
                available_space: Size {
                    width: AvailableSpace::Definite(context.column_width),
                    height: AvailableSpace::MaxContent,
                },
                vertical_margins_are_collapsible: TaffyLine::FALSE,
            },
        );
        if tree.fragment_tree.limit_exceeded
            || !output.size.width.is_finite()
            || !output.size.height.is_finite()
        {
            return None;
        }
        sizes.push(output.size);
    }
    let items: Vec<Item> = children
        .iter()
        .zip(&sizes)
        .map(|(&child, size)| {
            let (margin_top, margin_bottom) = block_margins(&tree.nodes[child]).unwrap_or_default();
            Item {
                height: size.height,
                margin_top,
                margin_bottom,
                splittable: !matches!(
                    tree.nodes[child].break_inside,
                    BreakInside::Avoid | BreakInside::AvoidColumn
                ),
            }
        })
        .collect();
    // One fragment for the container, the rest for the child pieces.
    let max_pieces = tree
        .fragment_tree
        .limit
        .saturating_sub(tree.fragment_tree.fragments.len())
        .saturating_sub(1);
    // A box that may not be cut falls back to the other strategies whole, so
    // running out of fragments while cutting it is not a layout failure.
    let unsliceable = children.iter().any(|&child| {
        let node = &tree.nodes[child];
        has_block_edge_decorations(node) || node.paints_whole_box_effects
    });
    let column_height = match definite_height {
        Some(height) => height,
        None if context.column_fill == ColumnFillValue::Auto => {
            let Some(height) = stacked_height(&items, max_pieces) else {
                tree.fragment_tree.limit_exceeded |= !unsliceable;
                return None;
            };
            height
        }
        None => balanced_height(&items, context.column_count, max_pieces),
    };
    // Columns are as tall as the container's content box (CSS Multi-column
    // 1, 2), so a minimum height makes them taller, whether the authored
    // height is definite or not.
    let column_height = column_height.max(minimum - origin.y - block_end_inset);
    if !column_height.is_finite() || column_height <= 0.0 {
        return None;
    }
    let Some(placements) = place(&items, column_height, max_pieces) else {
        tree.fragment_tree.limit_exceeded |= !unsliceable;
        return None;
    };
    // Painting draws every fragment's decorations whole, so a box with
    // block-axis borders or padding, or with effects sized from the whole
    // box, would repeat them at each break instead of slicing them (CSS
    // Fragmentation 3, 5.4). Such a box may still sit whole in one column.
    let repeats_decorations = children
        .iter()
        .zip(&placements.pieces)
        .any(|(&child, pieces)| {
            pieces.len() > 1 && {
                let node = &tree.nodes[child];
                has_block_edge_decorations(node) || node.paints_whole_box_effects
            }
        });
    if repeats_decorations {
        return None;
    }
    tree.fragment_tree.retire_previous_roots(parent);
    let used_height = if definite_height.is_some() {
        border_box.height
    } else {
        origin.y + column_height + block_end_inset
    };
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
