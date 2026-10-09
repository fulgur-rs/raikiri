use super::*;
use crate::fragment::{FragmentRect, LayoutFragment};

pub(super) fn supported(tree: &Document, parent: usize) -> Option<Vec<usize>> {
    if tree.nodes[parent].ifc.is_some() || tree.nodes[parent].style.direction != TaffyDirection::Ltr
    {
        return None;
    }
    let mut chain = Vec::new();
    let mut current = parent;
    let mut constrained = false;
    for _ in 0..128 {
        let mut children = tree.nodes[current].children.iter().copied().filter(|&id| {
            tree.nodes[id].is_in_document()
                && tree.nodes[id].kind() == NodeKind::Element
                && tree.nodes[id].style.display != Display::None
        });
        let child = children.next()?;
        if children.next().is_some() {
            return None;
        }
        let node = &tree.nodes[child];
        let defaults: taffy::Style = taffy::Style::default();
        if node.display != DisplayValue::Block
            || node.style.direction != TaffyDirection::Ltr
            || node.style.float.is_floated()
            || node.style.clear != defaults.clear
            || node.style.inset != Rect::auto()
            || node.style.padding != Rect::zero()
            || node.style.border != Rect::zero()
            || node.style.overflow.x == taffy::Overflow::Scroll
            || node.style.overflow.y == taffy::Overflow::Scroll
            || !node.style.size.height.is_auto()
            || !node.style.min_size.height.is_auto()
            || !node.style.max_size.height.is_auto()
            || node.break_inside != raikiri_style::property::BreakInside::Auto
            || node.break_before != BreakBetween::Auto
            || node.break_after != BreakBetween::Auto
            || node.has_before_or_after_content
            || node.multicol.is_some()
        {
            return None;
        }
        constrained |= node.style.margin != Rect::zero()
            || !node.multicol_auto_width
            || !node.style.min_size.width.is_auto()
            || !node.style.max_size.width.is_auto();
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

fn constrained_line_ranges(
    extents: &[(f32, f32)],
    height: f32,
    first_top: f32,
    orphans: usize,
    widows: usize,
) -> Option<Vec<(usize, usize)>> {
    use std::collections::VecDeque;

    #[derive(Clone, Copy)]
    struct Prefix {
        count: usize,
        previous: usize,
    }

    let length = extents.len();
    let last_start = length.checked_sub(widows)?;
    let mut prefixes: Vec<Option<Prefix>> = vec![None; length];
    let mut candidates: VecDeque<(usize, usize)> = VecDeque::new();
    let middle_minimum = orphans.max(widows);
    // Each prefix keeps the fewest preceding slices that satisfy both the
    // line-count minima and the physical height. The sliding minimum visits
    // each line once; variable line heights do not require trying every cut.
    for end in 1..length {
        if end >= orphans && extents[end - 1].1 - extents[0].0 <= height - first_top + f32::EPSILON
        {
            prefixes[end] = Some(Prefix {
                count: 1,
                previous: 0,
            });
        }
        if let Some(start) = end.checked_sub(middle_minimum)
            && let Some(prefix) = prefixes[start]
        {
            while candidates
                .back()
                .is_some_and(|&(_, count)| count >= prefix.count)
            {
                candidates.pop_back();
            }
            candidates.push_back((start, prefix.count));
        }
        while candidates.front().is_some_and(|&(start, _)| {
            extents[end - 1].1 - extents[start].0 > height + f32::EPSILON
        }) {
            candidates.pop_front();
        }
        if let Some(&(start, count)) = candidates.front() {
            let count = count.checked_add(1)?;
            if prefixes[end].is_none_or(|prefix| count < prefix.count) {
                prefixes[end] = Some(Prefix {
                    count,
                    previous: start,
                });
            }
        }
    }
    let (mut start, _) = prefixes
        .iter()
        .enumerate()
        .take(last_start + 1)
        .skip(1)
        .filter_map(|(start, prefix)| prefix.map(|prefix| (start, prefix)))
        .filter(|&(start, _)| extents[length - 1].1 - extents[start].0 <= height + f32::EPSILON)
        .min_by_key(|&(start, prefix)| (prefix.count, std::cmp::Reverse(start)))?;
    let mut ranges = vec![(start, length)];
    while start > 0 {
        let prefix = prefixes[start]?;
        ranges.push((prefix.previous, start));
        start = prefix.previous;
    }
    ranges.reverse();
    Some(ranges)
}

pub(super) fn layout(
    tree: &mut Document,
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
    let output = tree.compute_child_layout(
        TaffyNodeId::from(first),
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
                width: AvailableSpace::Definite(context.column_width),
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: TaffyLine::FALSE,
        },
    );
    let mut first_layout = tree.nodes[first].unrounded_layout;
    // Parent-owned used margins can still refer to the unfragmented width.
    // Resolve the outer margins against the column before placing this box.
    first_layout.margin = tree.nodes[first].style.margin.map(|margin| {
        margin
            .resolve_to_option(context.column_width, |pointer, basis| {
                tree.resolve_calc_value(pointer, basis)
            })
            .unwrap_or(0.0)
    });
    let left_auto = tree.nodes[first].style.margin.left.is_auto();
    let right_auto = tree.nodes[first].style.margin.right.is_auto();
    let leftover = (context.column_width
        - output.size.width
        - first_layout.margin.left
        - first_layout.margin.right)
        .max(0.0);
    let left = first_layout.margin.left
        + if left_auto {
            leftover / if right_auto { 2.0 } else { 1.0 }
        } else {
            0.0
        };
    first_layout.location = Point {
        x: origin.x + left,
        y: origin.y
            + output
                .top_margin
                .collapse_with_margin(first_layout.margin.top)
                .resolve(),
    };
    first_layout.size = output.size;
    tree.set_unrounded_layout(TaffyNodeId::from(first), &first_layout);
    let mut positions = Vec::with_capacity(chain.len());
    let mut position = Point::ZERO;
    let mut clip_left = 0.0_f32;
    let mut clip_right = context.column_width;
    for &id in chain {
        let layout = tree.nodes[id].unrounded_layout;
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
                _ => continue, // cov:ignore: guards exclude atomic/out-of-flow boxes; the IFC builder does not emit ruby annotations
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
    while start < extents.len() {
        let source_top = extents[start].0;
        let mut end = start
            + extents[start..]
                .partition_point(|extent| extent.1 - source_top <= height - top + f32::EPSILON);
        // A line taller than an empty fragmentainer must make progress.
        end = end.max(start + 1).min(extents.len());
        pieces.push(Piece {
            start,
            end,
            column: context.column_index.checked_add(pieces.len())?,
            top,
            bottom: top + extents[end - 1].1 - source_top,
        });
        start = end;
        top = 0.0;
    }
    let orphans = tree.nodes[leaf].paragraph_orphans.max(1);
    let widows = tree.nodes[leaf].paragraph_widows.max(1);
    let violates_minima = pieces.iter().enumerate().any(|(index, piece)| {
        let count = piece.end - piece.start;
        (index > 0 && count < widows) || (index + 1 < pieces.len() && count < orphans)
    });
    if violates_minima
        && let Some(ranges) = constrained_line_ranges(&extents, height, leaf_top, orphans, widows)
    {
        let mut constrained = Vec::with_capacity(ranges.len());
        for (order, (start, end)) in ranges.into_iter().enumerate() {
            let top = if order == 0 { leaf_top } else { 0.0 };
            constrained.push(Piece {
                start,
                end,
                column: context.column_index.checked_add(order)?,
                top,
                bottom: top + extents[end - 1].1 - extents[start].0,
            });
        }
        pieces = constrained;
    }
    // If no complete partition can satisfy the minima, retain progress and
    // satisfy the remaining boundaries wherever physical space permits.
    for boundary in (0..pieces.len().saturating_sub(1)).rev() {
        let minimum = if boundary + 2 == pieces.len() {
            widows
        } else {
            orphans.max(widows)
        };
        let needed = minimum.saturating_sub(pieces[boundary + 1].end - pieces[boundary + 1].start);
        // An intermediate slice can borrow from its predecessor on the next
        // iteration. Only the first slice must retain its orphans now.
        let retained = if boundary == 0 { orphans } else { 1 };
        let movable = (pieces[boundary].end - pieces[boundary].start).saturating_sub(retained);
        let mut moved = 0;
        while moved < needed.min(movable) {
            let new_start = pieces[boundary + 1].start - moved - 1;
            let bottom = extents[pieces[boundary + 1].end - 1].1 - extents[new_start].0;
            if bottom > height + f32::EPSILON {
                break;
            }
            moved += 1;
        }
        pieces[boundary].end -= moved;
        pieces[boundary + 1].start -= moved;
        for piece in &mut pieces[boundary..=boundary + 1] {
            piece.bottom = piece.top + extents[piece.end - 1].1 - extents[piece.start].0;
        }
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
            let source = tree.nodes[id].unrounded_layout;
            let y = if order == 0 {
                positions[depth].y
            } else {
                origin.y + if id == leaf { piece.top } else { 0.0 }
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
                fragmentainer_clip: Some(FragmentRect {
                    x: clip_x - parent_position.x,
                    y: origin.y - parent_position.y,
                    width: clip_right - clip_left,
                    height: height.max(piece.bottom),
                }),
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
