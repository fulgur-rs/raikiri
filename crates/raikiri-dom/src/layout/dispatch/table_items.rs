//! Reconcile stretched table sizes and overflowing flex cross-axis margins.
//!
//! Authored dimensions are measured by the table itself. This pass handles
//! external stretch assignments which still overwrite the returned grid size
//! or leave a reversed flex cross-start at the old position. It also records
//! the room that each line of a wrapping column container gives the items that
//! align by a baseline, for the container to align them in, and the line of
//! every item of a container that resolves `stretch` limits against its lines.

use std::collections::{HashMap, HashSet};

use super::inline_baseline;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{LayoutInput, LayoutOutput, NodeId, RunMode};
use crate::layout::style_views::ReadRole;
use crate::layout::taffy_adapter::algorithms::{
    FlexReplayCall, baseline_lines, flex_baseline_source, wrap_reverse,
};
use crate::node::FormattingContext;
use raikiri_layout_geometry::{Edges, Line, Point, Size};
use raikiri_layout_style::values::{
    AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignSelf, AlignmentSafety,
    Direction, Position, ResolveOrZero,
};
use rustc_hash::FxHashMap;

#[derive(Debug)]
pub(in crate::layout) struct Capture {
    container: usize,
    sizes: HashMap<usize, Measured>,
    calls: Option<Vec<FlexReplayCall>>,
}

#[derive(Debug, Clone, Copy)]
struct Measured {
    size: Size<f32>,
    offered: Size<f32>,
}

pub(super) fn record(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    output: &LayoutOutput,
) {
    let Some(parent) = tree
        .context
        .table_item_captures
        .last()
        .map(|capture| capture.container)
    else {
        return;
    };
    let idx = usize::from(node);
    if tree.taffy_parent_of(idx) != Some(parent) {
        return;
    }
    let table = tree.nodes[idx].formatting_context == FormattingContext::Table
        && tree.nodes[idx].layout_position() != Position::Absolute;
    let capture = tree
        .context
        .table_item_captures
        .last_mut()
        .expect("table capture remains active");
    if let Some(calls) = &mut capture.calls {
        calls.push(FlexReplayCall {
            node,
            inputs,
            output: output.clone(),
        });
    }
    if table && inputs.run_mode == RunMode::PerformLayout {
        capture.sizes.insert(
            idx,
            Measured {
                size: output.size,
                // Without an assignment there is no stretch delta to undo.
                offered: Size {
                    width: inputs.known_dimensions.width.unwrap_or(output.size.width),
                    height: inputs.known_dimensions.height.unwrap_or(output.size.height),
                },
            },
        );
    }
}

pub(super) fn layout(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    algorithm: fn(&mut LayoutSession<'_>, NodeId, LayoutInput) -> LayoutOutput,
) -> LayoutOutput {
    if inputs.run_mode != RunMode::PerformLayout {
        return algorithm(tree, node, inputs);
    }
    let idx = usize::from(node);
    let flex = tree.nodes[idx].formatting_context == FormattingContext::Flex;
    let parent = tree.query_style(
        idx,
        if flex {
            ReadRole::FlexContainer
        } else {
            ReadRole::GridContainer
        },
    );
    // The items of a column container that wraps its items and align by a
    // baseline: the container aligns them after the flex algorithm, in the
    // room each of its lines gives them.
    let wrapped_baselines = if flex {
        super::inline_baseline::wrapped_members(tree, idx)
    } else {
        HashSet::new()
    };
    let lines_requested = flex && tree.context.flex_line_requests.contains_key(&idx);
    let replay = flex
        && (lines_requested
            || wrap_reverse::required(tree, node)
            || !wrapped_baselines.is_empty()
            || (parent.flex_direction_is_row()
                && super::TaffyChildIter::new_for_session(tree, idx).any(|child| {
                    let margin = tree.query_style(child, ReadRole::FlexItem).margin();
                    margin.top.is_auto()
                        || margin.bottom.is_auto()
                        || (parent.flex_lines_are_reversed()
                            && (crate::layout::table::item_owns_height(tree, child)
                                || crate::layout::table::item_owns_width(tree, child)))
                })));
    tree.context.table_item_captures.push(Capture {
        container: idx,
        sizes: HashMap::new(),
        calls: replay.then(Vec::new),
    });
    let mut output = algorithm(tree, node, inputs);
    let capture = tree
        .context
        .table_item_captures
        .pop()
        .expect("table capture was pushed before layout");
    if flex {
        tree.context.flex_baseline_lines.remove(&idx);
        let mut flex_start_lines = false;
        if let Some(correction) = capture.calls.as_deref().and_then(|calls| {
            wrap_reverse::candidate(tree, node, inputs, calls, &output, |child| {
                Some(tree.nodes[child].layout)
            })
        }) {
            for (child, layout) in correction.layouts {
                tree.set_unrounded_layout(NodeId::from(child), &layout);
                tree.install_root_static_position_overrides(NodeId::from(child));
            }
            output = correction.output;
            flex_start_lines = true;
        }
        // The correction placed the lines at the start of the container.
        let content = flex_start_lines.then_some(AlignContent {
            keyword: AlignContentKeyword::FlexStart,
            safety: AlignmentSafety::Unsafe,
        });
        if lines_requested && let Some(calls) = capture.calls.as_deref() {
            record_item_lines(tree, node, inputs, calls, output.size, content);
        }
        if let Some(calls) = capture
            .calls
            .as_deref()
            .filter(|_| !wrapped_baselines.is_empty())
            && let Some(rooms) = baseline_lines::rooms(
                tree,
                node,
                inputs,
                calls,
                output.size,
                &wrapped_baselines,
                content,
            )
        {
            tree.context.flex_baseline_lines.insert(idx, rooms);
        }
        super::flex_cross_margins::normalize(
            tree,
            idx,
            inputs,
            capture.calls.as_deref(),
            &mut output,
        );
    }
    if !capture.sizes.is_empty() {
        if tree.nodes[idx].formatting_context == FormattingContext::Flex {
            normalize_flex(
                tree,
                idx,
                inputs,
                capture.sizes,
                capture.calls.as_deref(),
                &mut output,
            );
        } else {
            normalize_grid(tree, idx, inputs, capture.sizes, &mut output);
        }
    }
    output
}

/// Record in `flex_item_lines` the line each item that the flex container
/// `node` asked for sits on, from the `calls` its items reported, with the
/// lines aligned by `content` (the container's own alignment when `None`).
/// When the container may share free room between its lines, also record the
/// lines packed at its start and the room they leave free. Nothing is
/// recorded when the replays do not reproduce the container's `size`.
fn record_item_lines(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    calls: &[FlexReplayCall],
    size: Size<f32>,
    content: Option<AlignContent>,
) {
    if let Some(lines) = item_lines(tree, node, inputs, calls, size, content) {
        tree.context
            .flex_item_lines
            .insert(usize::from(node), lines);
    }
}

/// The lines [`record_item_lines`] records, or `None` when a replay does not
/// reproduce the container's `size`.
fn item_lines(
    tree: &mut LayoutSession<'_>,
    node: NodeId,
    inputs: LayoutInput,
    calls: &[FlexReplayCall],
    size: Size<f32>,
    content: Option<AlignContent>,
) -> Option<crate::layout::context::FlexItemLines> {
    let idx = usize::from(node);
    let members = tree.context.flex_line_requests.get(&idx).cloned()?;
    let style = tree.query_style(idx, ReadRole::FlexContainer);
    let row = style.flex_direction_is_row();
    let right_to_left = !row && style.direction() == Direction::Rtl;
    let reversed = style.flex_lines_are_reversed();
    let shares_room = content.is_none() && super::content_limits::shares_free_room(&style);
    let aligned =
        baseline_lines::cross_rooms(tree, node, inputs, calls, size, &members, content, row)?;
    let mut lines = crate::layout::context::FlexItemLines {
        aligned,
        packed: HashMap::new(),
        free_room: 0.0,
    };
    if shares_room {
        let content = |keyword| {
            Some(AlignContent {
                keyword,
                safety: AlignmentSafety::Unsafe,
            })
        };
        let packed = baseline_lines::cross_rooms(
            tree,
            node,
            inputs,
            calls,
            size,
            &members,
            content(AlignContentKeyword::FlexStart),
            row,
        )?;
        // Lines packed at the end of the container rather than at its start
        // move by the room they leave free. The near edge of an item at the
        // start of its line is the first edge of its packed room, or the
        // second in a right-to-left column container.
        let at_end = wrap_reverse::replay_with(
            tree,
            node,
            inputs,
            calls,
            wrap_reverse::Overrides {
                content: content(AlignContentKeyword::FlexEnd),
                items: Some((&members, AlignSelf::START)),
            },
        );
        let (output, at_end) = at_end?;
        let (child, room) = packed.iter().min_by_key(|&(&child, _)| child)?;
        let layout = at_end.get(child).filter(|_| output.size == size)?;
        let moved = if row {
            layout.location.y - layout.margin.top - room.edges.0
        } else if right_to_left {
            layout.location.x + layout.size.width + layout.margin.right - room.edges.1
        } else {
            layout.location.x - layout.margin.left - room.edges.0
        };
        // `flex-end` moves the lines toward the cross end, which is the
        // physical start when the lines run backwards.
        lines.free_room = if reversed ^ right_to_left {
            -moved
        } else {
            moved
        };
        lines.packed = packed;
    }
    Some(lines)
}

fn is_stretch(alignment: raikiri_layout_style::values::AlignSelf) -> bool {
    matches!(
        alignment.keyword,
        AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch
    )
}

fn normalize_grid(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    sizes: HashMap<usize, Measured>,
    output: &mut LayoutOutput,
) {
    let items: Vec<_> = super::TaffyChildIter::new_for_session(tree, idx)
        .filter(|&child| {
            !tree.nodes[child].is_display_none()
                && tree.nodes[child].layout_position() != Position::Absolute
        })
        .collect();
    let vertical_parent = super::grid_baseline::runs_vertically(tree, idx);
    let parent = tree.query_style(idx, ReadRole::GridContainer);
    let direction = parent.direction();
    let align_items = parent.align_items();
    let justify_items = parent.justify_items();
    let mut shifts = Vec::new();
    if !vertical_parent {
        settle_table_widths(tree, idx, &items, &sizes, &mut shifts);
    }
    for &child in &items {
        let Some(measured) = sizes.get(&child) else {
            continue;
        };
        let owns_width = crate::layout::table::item_owns_width(tree, child);
        if !owns_width && !crate::layout::table::item_owns_height(tree, child) {
            continue;
        }
        let inline = vertical_parent != owns_width;
        let reversed = inline && direction == Direction::Rtl;
        let default_align = if inline { justify_items } else { align_items };
        let style = crate::layout::table::item_style(tree, child, ReadRole::GridItem);
        let alignment = if inline {
            style.bridge_justify_self()
        } else {
            style.bridge_align_self()
        }
        .unwrap_or(default_align);
        let margin = style.margin();
        let auto_margin = if inline {
            margin.left.is_auto() || margin.right.is_auto()
        } else {
            margin.top.is_auto() || margin.bottom.is_auto()
        };
        if !is_stretch(alignment) || auto_margin {
            continue;
        }
        let mut layout = tree.nodes[child].layout;
        let (old, actual) = if inline {
            (layout.size.width, measured.size.width)
        } else {
            (layout.size.height, measured.size.height)
        };
        if actual >= old {
            continue;
        }
        let shift = if reversed { old - actual } else { 0.0 };
        if inline {
            layout.size.width = actual;
            layout.location.x += shift;
        } else {
            layout.size.height = actual;
        }
        tree.set_unrounded_layout(NodeId::from(child), &layout);
        shifts.push((child, Point { x: shift, y: 0.0 }));
    }
    finish(tree, idx, inputs, &items, &shifts, output);
}

/// Give the tables among the items of a horizontal grid the widths they were
/// laid out at.
///
/// Taffy sizes a grid item by the size it assigns, which a table answers with
/// its own: the stretch to the whole grid area and the `width`, `min-width`
/// and `max-width` it clamps by do not know that a keyword limit stands for
/// the columns' widths, nor that the table never gets narrower than its
/// narrowest columns. The item is then aligned in its area at the width the
/// table reports, where the alignment may fall back to the start edge (`safe`)
/// or the auto margins may have no room left, depending on that width.
fn settle_table_widths(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    items: &[usize],
    sizes: &HashMap<usize, Measured>,
    shifts: &mut Vec<(usize, Point<f32>)>,
) {
    let parent = tree.query_style(idx, ReadRole::GridContainer);
    let parent_direction = parent.direction();
    let rtl = parent_direction == Direction::Rtl;
    let justify_items = parent.justify_items();
    // The width of the grid area of each item, in the order of the items.
    let areas: Vec<Option<f32>> = {
        let grid = tree.context.grid_layout_info.get(&idx);
        (0..items.len())
            .map(|index| {
                grid.and_then(|grid| grid.item_inline_range(index))
                    .map(|(start, end)| end - start)
            })
            .collect()
    };
    for (&child, area) in items.iter().zip(areas) {
        let (Some(measured), Some(area)) = (sizes.get(&child), area) else {
            continue;
        };
        if !crate::layout::table::item_sized_by_columns(tree, child) {
            continue;
        }
        let mut layout = tree.nodes[child].layout;
        if (layout.size.width - measured.size.width).abs() <= WIDTH_EPSILON {
            continue;
        }
        let style = crate::layout::table::item_style(tree, child, ReadRole::GridItem);
        let margin = style.margin();
        let alignment = style
            .bridge_justify_self()
            .unwrap_or(justify_items)
            .resolve_self_relative(style.direction(), parent_direction, true);
        // The margins the item states; the auto ones take what the width
        // leaves.
        let stated = Line {
            start: (!margin.left.is_auto()).then_some(layout.margin.left),
            end: (!margin.right.is_auto()).then_some(layout.margin.right),
        };
        let before = align_in_area(area, layout.size.width, stated, alignment, rtl);
        let after = align_in_area(area, measured.size.width, stated, alignment, rtl);
        let shift = after.offset - before.offset;
        layout.size.width = measured.size.width;
        layout.location.x += shift;
        layout.margin.left = after.margin.start;
        layout.margin.right = after.margin.end;
        tree.set_unrounded_layout(NodeId::from(child), &layout);
        shifts.push((child, Point { x: shift, y: 0.0 }));
    }
}

/// Where an item sits along the inline axis of its grid area.
struct Aligned {
    /// The distance from the start of the area to the border box.
    offset: f32,
    /// The margins at the start and end of the item, the auto ones resolved.
    margin: Line<f32>,
}

/// Align an item of `width` in a grid area of `area` the way the grid
/// algorithm does: the auto margins share the room the item leaves, and a
/// `safe` alignment of an item that overflows its area gives way to `start`.
fn align_in_area(
    area: f32,
    width: f32,
    stated: Line<Option<f32>>,
    alignment: AlignItems,
    rtl: bool,
) -> Aligned {
    let area = area.max(0.0);
    let stated_sum = stated.start.unwrap_or(0.0) + stated.end.unwrap_or(0.0);
    let auto_count = u8::from(stated.start.is_none()) + u8::from(stated.end.is_none());
    let auto_size = if auto_count > 0 {
        (area - width - stated_sum).max(0.0) / f32::from(auto_count)
    } else {
        0.0
    };
    let margin = Line {
        start: stated.start.unwrap_or(auto_size),
        end: stated.end.unwrap_or(auto_size),
    };
    let overflows = width + stated_sum > area;
    let keyword = if alignment.safety == AlignmentSafety::Safe && overflows {
        AlignItemsKeyword::Start
    } else {
        alignment.keyword
    };
    let offset = match keyword {
        AlignItemsKeyword::Center => (area - width + margin.start - margin.end) / 2.0,
        AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd | AlignItemsKeyword::SelfEnd => {
            if rtl {
                margin.start
            } else {
                area - width - margin.end
            }
        }
        AlignItemsKeyword::Normal
        | AlignItemsKeyword::Stretch
        | AlignItemsKeyword::Start
        | AlignItemsKeyword::FlexStart
        | AlignItemsKeyword::SelfStart
        | AlignItemsKeyword::Baseline => {
            if rtl {
                area - width - margin.end
            } else {
                margin.start
            }
        }
    };
    Aligned { offset, margin }
}

/// Widths this close are the same width.
const WIDTH_EPSILON: f32 = 0.001;

fn normalize_flex(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    sizes: HashMap<usize, Measured>,
    calls: Option<&[FlexReplayCall]>,
    output: &mut LayoutOutput,
) {
    let parent = tree.query_style(idx, ReadRole::FlexContainer);
    let row = parent.flex_direction_is_row();
    let reversed =
        parent.flex_lines_are_reversed() ^ (!row && parent.direction() == Direction::Rtl);
    if !reversed {
        return;
    }
    let default_align = parent.align_items();
    let items: Vec<_> = super::TaffyChildIter::new_for_session(tree, idx)
        .filter(|&child| tree.nodes[child].layout_position() != Position::Absolute)
        .collect();
    let mut shifts = Vec::new();
    let mut layouts = Vec::new();
    for &child in &items {
        let Some(measured) = sizes.get(&child) else {
            continue;
        };
        if !crate::layout::table::item_owns_height(tree, child)
            && !crate::layout::table::item_owns_width(tree, child)
        {
            continue;
        }
        let offered = if row {
            measured.offered.height
        } else {
            measured.offered.width
        };
        let actual = if row {
            measured.size.height
        } else {
            measured.size.width
        };
        if actual >= offered {
            continue;
        }
        let style = crate::layout::table::item_style(tree, child, ReadRole::FlexItem);
        let margin = style.margin();
        let auto_margin = if row {
            margin.top.is_auto() || margin.bottom.is_auto()
        } else {
            margin.left.is_auto() || margin.right.is_auto()
        };
        if !is_stretch(style.align_self().unwrap_or(default_align)) || auto_margin {
            continue;
        }
        let delta = offered - actual;
        let mut layout = tree.nodes[child].layout;
        if row {
            layout.location.y += delta;
        } else {
            layout.location.x += delta;
        }
        layouts.push((child, layout));
        shifts.push((
            child,
            Point {
                x: if row { 0.0 } else { delta },
                y: if row { delta } else { 0.0 },
            },
        ));
    }
    if row && output.baselines.first.is_some() && !shifts.is_empty() {
        shift_first_baseline(
            tree,
            idx,
            inputs,
            calls.expect("reversed row table layout records its callbacks"),
            &shifts,
            output,
        );
    }
    for (child, layout) in layouts {
        tree.set_unrounded_layout(NodeId::from(child), &layout);
    }
    finish(tree, idx, inputs, &items, &shifts, output);
}

/// Shift only the baseline contributed by one of the moved flex items.
/// Placement and read-only baseline probes share the same replay.
pub(in crate::layout) fn shift_first_baseline(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    calls: &[FlexReplayCall],
    shifts: &[(usize, Point<f32>)],
    output: &mut LayoutOutput,
) {
    let moved: Vec<_> = shifts.iter().map(|&(child, _)| child).collect();
    match flex_baseline_source(tree, NodeId::from(idx), inputs, calls, &moved) {
        Ok(Some(source)) => {
            let (_, shift) = shifts
                .iter()
                .find(|&&(child, _)| child == source)
                .expect("replay only identifies a moved item");
            output.baselines.first = output.baselines.first.map(|baseline| baseline + shift.y);
        }
        Ok(None) => {}
        Err(message) => tree.context.table_layout_error = Some(message.to_owned()),
    }
}

#[cfg(test)]
mod tests;
pub(super) fn finish(
    tree: &mut LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    items: &[usize],
    shifts: &[(usize, Point<f32>)],
    output: &mut LayoutOutput,
) {
    if shifts.is_empty() {
        return;
    }
    // Local out-of-flow layout runs after this pass. Only the candidates
    // already translated by the container need to follow their moved item now.
    let mut moved: FxHashMap<usize, Point<f32>> = FxHashMap::default();
    for &(item, shift) in shifts {
        let total = moved.entry(item).or_insert(Point { x: 0.0, y: 0.0 });
        total.x += shift.x;
        total.y += shift.y;
    }
    output.oof_candidates.for_each_mut(|candidate| {
        let node = usize::from(candidate.node);
        let Some(shift) =
            inline_baseline::item_holding(tree, idx, node).and_then(|item| moved.get(&item))
        else {
            return;
        };
        candidate.static_position.x.area.start += shift.x;
        candidate.static_position.x.area.end += shift.x;
        candidate.static_position.y.area.start += shift.y;
        candidate.static_position.y.area.end += shift.y;
    });
    output.scrollable_overflow_rect = overflow(tree, idx, inputs, items, output.size);
}

fn overflow(
    tree: &LayoutSession<'_>,
    idx: usize,
    inputs: LayoutInput,
    items: &[usize],
    size: Size<f32>,
) -> Edges<f32> {
    let flex = tree.nodes[idx].formatting_context == FormattingContext::Flex;
    let style = tree.query_style(
        idx,
        if flex {
            ReadRole::FlexContainer
        } else {
            ReadRole::GridContainer
        },
    );
    let direction = style.direction();
    let border = style.border().resolve_or_zero(inputs.parent_size.width);
    let padding = style.padding().resolve_or_zero(inputs.parent_size.width);
    let scrolls =
        style.overflow().x.is_scroll_container() || style.overflow().y.is_scroll_container();
    let mut rect = Edges::ZERO;
    for &child in items {
        let layout = tree.nodes[child].layout;
        let style = tree.query_style(
            child,
            if flex {
                ReadRole::FlexItem
            } else {
                ReadRole::GridItem
            },
        );
        let location = Point {
            x: if direction == Direction::Rtl {
                size.width - layout.location.x - layout.size.width - border.right
            } else {
                layout.location.x - border.left
            },
            y: layout.location.y - border.top,
        };
        rect = rect.union(super::vertical_oof::overflow_contribution(
            location,
            layout.size,
            layout.scrollable_overflow_rect,
            style.overflow(),
            style.contain(),
            scrolls,
        ));
    }
    if scrolls {
        rect.right += if direction == Direction::Rtl {
            padding.left
        } else {
            padding.right
        };
        rect.bottom += padding.bottom;
    }
    rect
}
