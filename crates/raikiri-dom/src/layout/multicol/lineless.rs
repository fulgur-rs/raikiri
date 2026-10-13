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

/// Fill columns of `column_height` with `items` in order. A splittable box
/// continues at the top of the next column; a box that avoids breaks moves
/// to the next column when it does not fit below earlier content, and
/// overflows a column it starts at the top of (CSS Fragmentation 3, 4.4).
/// Adjoining sibling margins collapse, and a margin at a break is truncated
/// (CSS Fragmentation 3, 5.2).
fn place(items: &[Item], column_height: f32) -> Vec<Vec<Piece>> {
    let mut column = 0usize;
    let mut cursor = 0.0_f32;
    let mut pending_margin = 0.0_f32;
    let mut placements = Vec::with_capacity(items.len());
    for item in items {
        let mut pieces = Vec::new();
        let mut gap = pending_margin.max(item.margin_top);
        let fits_after_gap = if item.splittable {
            cursor + gap < column_height - EPSILON
        } else {
            cursor + gap + item.height <= column_height + EPSILON
        };
        if cursor > 0.0 && !fits_after_gap && (item.height > 0.0 || !item.splittable) {
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
            pieces.push(Piece {
                column,
                top: cursor,
                height: item.height,
            });
            cursor += item.height;
        }
        placements.push(pieces);
    }
    placements
}

fn columns_used(placements: &[Vec<Piece>]) -> usize {
    placements
        .iter()
        .flatten()
        .map(|piece| piece.column + 1)
        .max()
        .unwrap_or(0)
}

/// The smallest column height that fits `items` in `column_count` columns.
/// Content that may break anywhere shares the columns equally; boxes that
/// avoid breaks can need taller columns (CSS Multi-column 1, 7.1).
fn balanced_height(items: &[Item], column_count: usize) -> f32 {
    let total: f32 = items
        .iter()
        .map(|item| item.height + item.margin_top + item.margin_bottom)
        .sum();
    let tallest = items
        .iter()
        .filter(|item| !item.splittable)
        .map(|item| item.height)
        .fold(0.0_f32, f32::max);
    let mut low = (total / column_count as f32).max(tallest);
    if columns_used(&place(items, low)) <= column_count {
        return low;
    }
    let mut high = total;
    // Each probe places the children once; the bound keeps the search linear
    // in the number of children.
    for _ in 0..24 {
        let middle = (low + high) / 2.0;
        if columns_used(&place(items, middle)) <= column_count {
            high = middle;
        } else {
            low = middle;
        }
    }
    high
}

/// Lay out `children` (from [`supported`]) as one stream cut into columns.
/// Returns the used border-box height of the container.
#[allow(clippy::too_many_arguments)]
pub(super) fn layout(
    tree: &mut Document,
    parent: usize,
    children: &[usize],
    context: FragmentationContext,
    border_box: Size<f32>,
    origin: Point<f32>,
    block_end_inset: f32,
) -> Option<f32> {
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
                    height: context.available_height,
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
    let total: f32 = items
        .iter()
        .map(|item| item.height + item.margin_top + item.margin_bottom)
        .sum();
    let column_height = match context.available_height {
        Some(height) => height,
        None if context.column_fill == ColumnFillValue::Auto => total,
        None => balanced_height(&items, context.column_count),
    };
    if !column_height.is_finite() || column_height <= 0.0 || !total.is_finite() {
        return None;
    }
    let placements = place(&items, column_height);
    let fragment_count = placements
        .iter()
        .map(Vec::len)
        .try_fold(1usize, usize::checked_add)?;
    if fragment_count
        > tree
            .fragment_tree
            .limit
            .saturating_sub(tree.fragment_tree.fragments.len())
    {
        tree.fragment_tree.limit_exceeded = true;
        return None;
    }
    tree.fragment_tree.retire_previous_roots(parent);
    let used_height = if context.available_height.is_some() {
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
    for (order, (&child, pieces)) in children.iter().zip(&placements).enumerate() {
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
mod tests {
    use super::{Item, balanced_height, place};

    fn shape(items: &[Item], height: f32) -> Vec<Vec<(usize, f32, f32)>> {
        place(items, height)
            .iter()
            .map(|pieces| {
                pieces
                    .iter()
                    .map(|piece| (piece.column, piece.top, piece.height))
                    .collect()
            })
            .collect()
    }

    fn split(height: f32) -> Item {
        Item {
            height,
            margin_top: 0.0,
            margin_bottom: 0.0,
            splittable: true,
        }
    }

    fn avoid(height: f32) -> Item {
        Item {
            splittable: false,
            ..split(height)
        }
    }

    #[test]
    fn splittable_boxes_continue_in_the_next_column() {
        assert_eq!(
            shape(&[split(50.0), split(270.0)], 100.0),
            vec![
                vec![(0, 0.0, 50.0)],
                vec![
                    (0, 50.0, 50.0),
                    (1, 0.0, 100.0),
                    (2, 0.0, 100.0),
                    (3, 0.0, 20.0)
                ]
            ]
        );
    }

    #[test]
    fn a_box_ending_at_a_column_edge_stays_in_that_column() {
        assert_eq!(
            shape(&[split(100.0), split(50.0)], 100.0),
            vec![vec![(0, 0.0, 100.0)], vec![(1, 0.0, 50.0)]]
        );
    }

    #[test]
    fn boxes_avoiding_breaks_move_or_overflow_whole() {
        assert_eq!(
            shape(&[avoid(20.0), avoid(40.0), avoid(40.0)], 30.0),
            vec![
                vec![(0, 0.0, 20.0)],
                vec![(1, 0.0, 40.0)],
                vec![(2, 0.0, 40.0)]
            ]
        );
    }

    #[test]
    fn sibling_margins_collapse_and_truncate_at_a_break() {
        let first = Item {
            margin_bottom: 20.0,
            ..avoid(40.0)
        };
        let second = Item {
            margin_top: 10.0,
            ..avoid(10.0)
        };
        // 40 + max(20, 10) + 10 = 70 fits a 70px column.
        assert_eq!(
            shape(&[first, second], 70.0),
            vec![vec![(0, 0.0, 40.0)], vec![(0, 60.0, 10.0)]]
        );
        // In a 60px column the second box moves, and loses its margin.
        assert_eq!(
            shape(&[first, second], 60.0),
            vec![vec![(0, 0.0, 40.0)], vec![(1, 0.0, 10.0)]]
        );
    }

    #[test]
    fn balancing_shares_breakable_content_and_respects_avoidance() {
        assert_eq!(balanced_height(&[split(400.0)], 4), 100.0);
        assert_eq!(balanced_height(&[avoid(40.0), avoid(40.0)], 3), 40.0);
        // Three 30px boxes that avoid breaks need 60px in two columns.
        let height = balanced_height(&[avoid(30.0), avoid(30.0), avoid(30.0)], 2);
        assert!((height - 60.0).abs() < 0.01, "{height}");
    }
}
