use super::*;
use crate::fragment::{FragmentRect, LayoutFragment};
use crate::layout::context::LayoutSession;
use raikiri_layout_geometry::Edges;
use raikiri_layout_style::values::MaybeResolve;

pub(super) fn supported(tree: &LayoutSession<'_>, parent: usize) -> Option<Vec<usize>> {
    if tree.nodes[parent].ifc.is_some() {
        return None;
    }
    // Every box of the chain runs in the container's direction, so one
    // alignment rule places the whole chain in its columns.
    let direction = style_of(tree, parent).direction();
    let mut chain = Vec::new();
    let mut current = parent;
    let mut constrained = false;
    for _ in 0..128 {
        let mut children = flow_children(tree, current).into_iter().filter(|&id| {
            tree.nodes[id].is_in_document()
                && tree.nodes[id].kind() == NodeKind::Element
                && !tree.nodes[id].is_display_none()
        });
        let child = children.next()?;
        if children.next().is_some() {
            return None;
        }
        let node = layout_node(tree, child);
        let style = style_of(tree, child);
        let overflow = style.overflow();
        // The chain is placed from the locations block flow gave its boxes,
        // so a block-axis offset stays with block flow.
        if node.display() != DisplayValue::Block
            || style.direction() != direction
            || style.float().is_floated()
            || style.clear() != Clear::None
            || !super::insets_are_a_relative_inline_offset(tree, child)
            || style.padding() != Edges::splat(LengthPercentage::ZERO)
            || style.border() != Edges::splat(LengthPercentage::ZERO)
            || overflow.x == Overflow::Scroll
            || overflow.y == Overflow::Scroll
            || !style.size().height.is_auto()
            || !style.min_size().height.is_auto()
            || !style.max_size().height.is_auto()
            || node.break_inside != raikiri_style::property::BreakInside::Auto
            || sets_break_in_columns(node.break_before)
            || sets_break_in_columns(node.break_after)
            || node.has_before_or_after_content
            || node.multicol.is_some()
        {
            return None;
        }
        constrained |= style.margin() != Edges::splat(LengthPercentageAuto::ZERO)
            || !node.multicol_auto_width
            || !style.min_size().width.is_auto()
            || !style.max_size().width.is_auto();
        chain.push(child);
        if node.ifc.is_some() {
            return (constrained
                && node.ifc_writing_mode() == Some(shodo::geometry::WritingMode::HorizontalTb)
                && node.ifc_boxes().is_empty()
                && !multicol_subtree_has_float(tree, child))
            .then_some(chain);
        }
        current = child;
    }
    None
}

struct Piece {
    start: usize,
    end: usize,
    column: usize,
    top: f32,
    bottom: f32,
}

/// Where the piece of a paragraph of `count` lines that starts at line
/// `start` ends when `fitting` is the end of the lines its column holds: the
/// break goes up to where the `widows` follow it, as long as the piece keeps
/// its `orphans`. Orphans win over widows: a piece too short for both ends
/// past its orphans, only as far as they need, or at the column end when the
/// widows begin at or before its start. Only the lines after the last break
/// count for the widows, so a piece between two breaks may hold fewer lines
/// than them (Chromium).
pub(super) fn piece_end(
    start: usize,
    fitting: usize,
    count: usize,
    (orphans, widows): (usize, usize),
) -> usize {
    if fitting >= count {
        return count;
    }
    let cut = count.saturating_sub(widows);
    let kept = start + orphans;
    if fitting.min(cut) >= kept {
        fitting.min(cut)
    } else if start > 0 && cut <= start || fitting < kept {
        fitting
    } else {
        fitting.min(cut.max(kept))
    }
}

/// The height of the balanced columns that hold the lines `extents` from
/// `start` on, the first column with `top` taken already: the least height,
/// up to `height`, at which the lines broken one column at a time fit the
/// columns left. Like Chromium, the columns are first made tall enough to
/// hold the orphans and the widows of the paragraph, where `height` allows.
fn balanced_height(
    extents: &[(f32, f32)],
    (start, top): (usize, f32),
    height: f32,
    context: FragmentationContext,
    (orphans, widows): (usize, usize),
    break_lines: &dyn Fn(usize, f32, f32) -> Vec<(usize, usize, f32)>,
) -> f32 {
    let columns = context
        .column_count
        .saturating_sub(context.column_index)
        .max(1);
    let count = extents.len();
    let first = (start + orphans.min(count - start)).max(start + 1);
    let last = count - widows.clamp(1, count - start);
    // A line never breaks, so the columns are at least as tall as the
    // tallest of them.
    let tallest = extents[start..]
        .iter()
        .map(|(line_top, bottom)| bottom - line_top)
        .fold(top + extents[start].1 - extents[start].0, f32::max);
    let floor = (top + extents[first - 1].1 - extents[start].0)
        .max(extents[count - 1].1 - extents[last].0)
        .max(tallest)
        .min(height);
    let fits = |candidate: f32| break_lines(start, top, candidate).len() <= columns;
    if fits(floor) || !fits(height) {
        return if fits(floor) { floor } else { height };
    }
    // The count of columns falls as they grow, so the least height that
    // fits lies between one that does not and one that does.
    let (mut low, mut high) = (floor, height);
    for _ in 0..32 {
        let middle = (low + high) / 2.0;
        if fits(middle) {
            high = middle;
        } else {
            low = middle;
        }
    }
    high
}

pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    parent: usize,
    chain: &[usize],
    context: FragmentationContext,
    border_box: Size<f32>,
    origin: Point<f32>,
) -> Option<f32> {
    let height = context
        .available_height
        .filter(|value| value.is_finite() && *value > 0.0)?;
    let first = *chain.first()?;
    let leaf = *chain.last()?;
    // The containing block supplies space, not a forced used width. Taffy
    // resolves authored, percentage, min/max and auto widths against it.
    for &id in chain {
        tree.nodes[id].cache.clear();
    }
    // A button fits its content within the room its margins leave (HTML
    // Standard 15.5.3); block flow takes the margins off for a box it
    // stretches, but not for one that shrinks.
    let available_width = if layout_node(tree, first).is_html_button() {
        super::column_room(&style_of(tree, first), context.column_width)
    } else {
        context.column_width
    };
    let output = tree.compute_child_layout(
        NodeId::from(first),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size::NONE,
            known_dimensions_are_definite: Size {
                width: false,
                height: false,
            },
            parent_size: Size {
                width: Some(context.column_width),
                height: Some(height),
            },
            available_space: Size {
                width: AvailableSpace::Definite(available_width),
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    let mut first_layout = tree.nodes[first].layout;
    // Parent-owned used margins can still refer to the unfragmented width.
    // Resolve the outer margins against the column before placing this box.
    let margin_style = style_of(tree, first).margin();
    first_layout.margin =
        margin_style.map(|margin| margin.maybe_resolve(context.column_width).unwrap_or(0.0));
    let left_auto = margin_style.left.is_auto();
    let right_auto = margin_style.right.is_auto();
    let room = context.column_width
        - output.size.width
        - first_layout.margin.left
        - first_layout.margin.right;
    let leftover = room.max(0.0);
    // CSS 2.1 section 10.3.3: a box that overflows its column counts auto
    // margins as zero, and a box with no auto margin is over-constrained.
    // Either way the margin on the end side of the container's direction is
    // the one that gives, so in a right-to-left container the box keeps its
    // right margin and sits at the right edge of its column.
    let over_constrained = !left_auto && !right_auto || room < 0.0;
    let left = if context.rtl && over_constrained {
        context.column_width - output.size.width - first_layout.margin.right
    } else {
        first_layout.margin.left
            + if left_auto {
                leftover / if right_auto { 2.0 } else { 1.0 }
            } else {
                0.0
            }
    };
    // Columns progress from the right edge in a right-to-left container.
    let column_x = if context.rtl {
        context.column_offset_x(context.column_index)
    } else {
        0.0
    };
    first_layout.location = Point {
        x: origin.x + column_x + left,
        y: origin.y
            + output
                .top_margin
                .collapse_with_margin(first_layout.margin.top)
                .resolve(),
    };
    first_layout.size = output.size;
    tree.set_unrounded_layout(NodeId::from(first), &first_layout);
    let mut positions = Vec::with_capacity(chain.len());
    let mut position = Point::ZERO;
    let mut clip_left = column_x;
    let mut clip_right = column_x + context.column_width;
    for &id in chain {
        let layout = tree.nodes[id].layout;
        position.x += layout.location.x;
        position.y += layout.location.y;
        if !position.x.is_finite() || !position.y.is_finite() || !layout.size.width.is_finite() {
            return None; // cov:ignore: saved Taffy layouts are sanitized; at most 128 bounded coordinates are summed
        }
        clip_left = clip_left.min(position.x - origin.x);
        clip_right = clip_right.max(position.x - origin.x + layout.size.width);
        positions.push(position);
    }
    let leaf_top = positions.last()?.y - origin.y;
    let root = tree.nodes[leaf].ifc.as_ref()?;
    let lines = root.lines.as_ref()?;
    let inline_origin = positions.last()?.x - origin.x;
    for line in lines.lines.iter() {
        for fragment in line.fragments() {
            let (left, width) = match fragment {
                shodo::Fragment::GlyphRun(run) => (run.inline_start(), run.inline_size()),
                shodo::Fragment::InlineBox(box_) => (box_.rect.inline_start, box_.rect.inline_size),
                // A ruby's annotations stay within the extent of its base
                // runs and inline box, which shodo widens to hold them.
                shodo::Fragment::RubyAnnotation(_) => continue,
                _ => continue, // cov:ignore: guards exclude atomic/out-of-flow boxes
            };
            clip_left = clip_left.min(inline_origin + left);
            clip_right = clip_right.max(inline_origin + left + width);
        }
    }
    let extents: Vec<_> = lines
        .lines
        .iter()
        .map(|line| (line.block_offset(), line.block_offset() + line.block_size()))
        .collect();
    if extents
        .iter()
        .any(|(top, bottom)| !top.is_finite() || !bottom.is_finite())
    {
        return None; // cov:ignore: shodo sanitizes line constraints and stores bounded line sizes; finite offsets cannot overflow f32 here
    }
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut top = leaf_top;
    // The depth of the button that left the column the chain starts in,
    // with how far up the move took it.
    let mut moved = None;
    // A button is monolithic: a column end never breaks between its lines.
    // One whose border box fits a column but not below the space above it
    // goes whole to the next column, which truncates that space; one taller
    // than a column stays whole where it is, past the column end. What is
    // inside it keeps its place in it.
    if !extents.is_empty()
        && let Some(button) = chain
            .iter()
            .position(|&id| layout_node(tree, id).is_html_button())
    {
        let size = extents[extents.len() - 1].1 - extents[0].0;
        let button_top = positions[button].y - origin.y;
        let button_height = tree.nodes[chain[button]].layout.size.height;
        let next = button_top > 0.0
            && button_top + button_height > height + f32::EPSILON
            && button_height <= height;
        if next {
            top -= button_top;
            moved = Some((button, button_top));
        }
        pieces.push(Piece {
            start,
            end: extents.len(),
            column: context.column_index.checked_add(usize::from(next))?,
            top,
            bottom: top + size,
        });
        start = extents.len();
    }
    let orphans = tree.nodes[leaf].paragraph_orphans.max(1);
    let paragraph_widows = tree.nodes[leaf].paragraph_widows;
    // The lines from `start` on, broken greedily into columns `height` tall,
    // the first of which has `top` taken already.
    let break_lines = |start: usize, top: f32, height: f32| {
        // Widows taller than a column cannot follow a break together, and
        // are dropped, as Chromium drops them.
        let widows = paragraph_widows.clamp(1, extents.len().max(1));
        let widows = if extents.len() >= widows
            && extents.last().is_some_and(|last| {
                last.1 - extents[extents.len() - widows].0 <= height + f32::EPSILON
            }) {
            widows
        } else {
            1
        };
        let mut ranges = Vec::new();
        let (mut start, mut top) = (start, top);
        while start < extents.len() {
            let source_top = extents[start].0;
            let fitting = start
                + extents[start..]
                    .partition_point(|extent| extent.1 - source_top <= height - top + f32::EPSILON);
            let end = piece_end(start, fitting, extents.len(), (orphans, widows));
            // A line taller than an empty fragmentainer must make progress.
            let end = end.max(start + 1).min(extents.len());
            ranges.push((start, end, top));
            start = end;
            top = 0.0;
        }
        ranges
    };
    // Balanced columns break the lines at their own height; the columns
    // keep the height of the container.
    let column_height = if context.column_fill == ColumnFillValue::Balance && start < extents.len()
    {
        balanced_height(
            &extents,
            (start, top),
            height,
            context,
            (orphans, paragraph_widows),
            &break_lines,
        )
    } else {
        height
    };
    for (start, end, top) in break_lines(start, top, column_height) {
        pieces.push(Piece {
            start,
            end,
            column: context.column_index.checked_add(pieces.len())?,
            top,
            bottom: top + extents[end - 1].1 - extents[start].0,
        });
    }
    let required = pieces.len().checked_mul(chain.len())?.checked_add(1)?;
    if required
        > tree
            .fragment_tree
            .limit
            .saturating_sub(tree.fragment_tree.fragments.len())
    {
        // Do not install ranges whose complete physical origins cannot be kept.
        tree.fragment_tree.limit_exceeded = true;
        return None;
    }
    let fragments = pieces
        .iter()
        .map(|piece| MulticolTextFragment {
            line_start: piece.start,
            line_end: piece.end,
            fragmentainer: piece.column,
            x: context.column_offset_x(piece.column)
                - context.column_offset_x(context.column_index),
            y: piece.top - leaf_top,
        })
        .collect();
    let root = tree.nodes[leaf].ifc.as_mut()?;
    root.multicol_fragments = Some(fragments);
    root.multicol_fragment_origins_recorded = true;
    tree.fragment_tree.retire_previous_roots(parent);
    let container = tree.fragment_tree.try_push(LayoutFragment {
        node_id: parent,
        parent: None,
        fragmentainer: context.column_index,
        rect: FragmentRect {
            x: context.origin_x,
            y: context.origin_y,
            width: border_box.width,
            height: border_box.height,
        },
        fragmentainer_clip: None,
        fragment_index: 0,
        fragment_count: 1,
        line_start: None,
        line_end: None,
    })?;
    for (order, piece) in pieces.iter().enumerate() {
        let dx =
            context.column_offset_x(piece.column) - context.column_offset_x(context.column_index);
        let clip_x = origin.x + dx + clip_left;
        let mut fragment_parent = container;
        let mut parent_position = Point::ZERO;
        for (depth, &id) in chain.iter().enumerate() {
            let source = tree.nodes[id].layout;
            let y = match moved {
                Some((button, shift)) if depth >= button => positions[depth].y - shift,
                Some(_) => origin.y,
                None if order == 0 => positions[depth].y,
                None => origin.y + if id == leaf { piece.top } else { 0.0 },
            };
            let x = positions[depth].x + dx;
            let bottom = if order + 1 < pieces.len() {
                origin.y + height.max(piece.bottom)
            } else {
                // Keep trailing content such as a child's non-collapsing
                // bottom margin inside the final ancestor slice.
                let tail = positions[depth].y + source.size.height
                    - (positions.last()?.y + extents.last()?.1);
                origin.y + piece.bottom + tail
            };
            fragment_parent = tree.fragment_tree.try_push(LayoutFragment {
                node_id: id,
                parent: Some(fragment_parent),
                fragmentainer: piece.column,
                rect: FragmentRect {
                    x: x - parent_position.x,
                    y: y - parent_position.y,
                    width: source.size.width,
                    height: (bottom - y).max(0.0),
                },
                fragmentainer_clip: Some(
                    FragmentRect {
                        x: clip_x - parent_position.x,
                        y: origin.y - parent_position.y,
                        width: clip_right - clip_left,
                        height: height.max(piece.bottom),
                    }
                    .into(),
                ),
                fragment_index: order,
                fragment_count: pieces.len(),
                line_start: (id == leaf).then_some(piece.start),
                line_end: (id == leaf).then_some(piece.end),
            })?;
            parent_position = Point { x, y };
        }
    }
    Some(border_box.height)
}

#[cfg(test)]
mod tests;
