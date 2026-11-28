//! Column spanners in the paragraph of a multicol container.
//!
//! A multicol container that holds text or inline boxes next to a
//! `column-span: all` block is the root of one paragraph, and the spanner is a
//! block box of it. The lines of the paragraph are broken at the width of a
//! column. A spanner ends the lines above it and starts the lines below it, so
//! the lines between two spanners form a group that is balanced over the
//! columns on its own, and the spanner crosses all of them below the group
//! above (CSS Multi-column Layout 1, 6.1).
//!
//! The other blocks of the paragraph stand among its lines, in anonymous
//! blocks of their own (CSS 2, 9.2.1.1): they are laid out at the width of a
//! column with the lines, and go with them to the columns of their group.

use super::*;
use crate::fragment::MulticolGroup;
use crate::layout::ifc::boxes::{IfcBoxKind, place_block_across};
use crate::layout::ifc::flow::FlowGeometry;
use crate::layout::ifc::root::{IfcLines, IfcRoot};
use crate::layout::protocol::CollapsibleMarginSet;

/// Whether the paragraph `root` of the multicol container `idx` is laid out in
/// groups of columns around its blocks: it holds at least one block, and no
/// float or out-of-flow box, which would need a place in the columns of its
/// own. A block that is not a spanner moves whole to a column, so it may
/// neither force a break in the columns, nor hold a float that the lines
/// after it would flow around where it was first laid out.
pub(crate) fn supports(tree: &LayoutSession<'_>, idx: usize, root: &IfcRoot) -> bool {
    tree.context.fragmentation_stack.is_empty()
        && tree.nodes[idx]
            .multicol
            .is_some_and(|style| style.horizontal)
        && root.generated_blocks.is_empty()
        && root.boxes.iter().any(|box_| box_.kind == IfcBoxKind::Block)
        && root.boxes.iter().all(|box_| match box_.kind {
            IfcBoxKind::Atomic | IfcBoxKind::Block => true,
            IfcBoxKind::Float | IfcBoxKind::OutOfFlow => false,
        })
        && !multicol_has_out_of_flow_descendant(tree, idx)
        && root.boxes.iter().all(|box_| {
            box_.kind != IfcBoxKind::Block
                || spanning::is_spanner(tree, box_.node)
                || moves_whole(tree, box_.node)
        })
}

/// Whether the block `node` among the lines of a paragraph can move to a
/// column as one piece: it forces no break in the columns, and no float
/// inside it shares the float context of the paragraph.
fn moves_whole(tree: &LayoutSession<'_>, node: usize) -> bool {
    let own = &tree.nodes[node];
    if sets_break_in_columns(own.break_before) || sets_break_in_columns(own.break_after) {
        return false;
    }
    let mut pending = own.children.clone();
    while let Some(child) = pending.pop() {
        if is_floated(tree, child) {
            return false;
        }
        pending.extend_from_slice(&tree.nodes[child].children);
    }
    true
}

/// Whether the block `node` of a paragraph is a column spanner, which the
/// lines of the paragraph leave room for only once they are in columns.
pub(crate) fn is_spanner(tree: &LayoutSession<'_>, node: usize) -> bool {
    spanning::is_spanner(tree, node)
}

/// The columns of the groups of a paragraph, the height they and the spanners
/// between them need, and the baselines the container exports.
pub(crate) struct Groups {
    pub(crate) fragments: Vec<MulticolTextFragment>,
    pub(crate) height: f32,
    /// The first baseline of the first line or spanner that has one, from the
    /// top of the content box.
    pub(crate) first_baseline: Option<f32>,
    /// The baseline of the last line, measured like `first_baseline`.
    pub(crate) last_baseline: Option<f32>,
}

/// The alphabetic baseline of the line `index` of `lines`, from the top of the
/// content box, given the columns that hold it. The lines of a paragraph that
/// is laid out in groups are horizontal, which stand on the alphabetic
/// baseline.
fn line_baseline(lines: &IfcLines, columns: &[MulticolTextFragment], index: usize) -> Option<f32> {
    let column = columns
        .iter()
        .find(|column| (column.line_start..column.line_end).contains(&index))?;
    let first = lines.lines.get(column.line_start)?;
    let line = lines.lines.get(index)?;
    Some(
        column.y + line.block_offset() - first.block_offset()
            + line.baseline(shodo::geometry::BaselineKind::Alphabetic),
    )
}

/// Place the groups of the lines of the paragraph `lines`, broken at the width
/// of a column, in columns, and the spanners that separate them across
/// `width`, the width of the content box of the container.
pub(crate) fn arrange(
    tree: &mut LayoutSession<'_>,
    owner: usize,
    lines: &IfcLines,
    context: FragmentationContext,
    geometry: FlowGeometry,
    width: f32,
    perform: bool,
) -> Groups {
    let across = FlowGeometry {
        width,
        content_size: shodo::geometry::PhysicalSize {
            width,
            ..geometry.content_size
        },
        blocks_apart: false,
        ..geometry
    };
    let mut fragments = Vec::new();
    // The rules between the columns run along each group, not across the
    // spanners between them.
    if perform {
        tree.nodes[owner].multicol_groups.clear();
    }
    let mut y = 0.0;
    let mut pending = CollapsibleMarginSet::ZERO;
    let mut first_baseline = None;
    let mut last_baseline = None;
    let mut start = 0;
    // The blocks between the lines of the group being collected, with the
    // index of the first line after each.
    let mut blocks = Vec::new();
    for end in 0..=lines.block_line_starts.len() {
        let entry = lines.block_line_starts.get(end).copied();
        if let Some((node, after)) = entry
            && !spanning::is_spanner(tree, node)
        {
            blocks.push((node, after, end));
            continue;
        }
        let spanner = entry.map(|(node, _)| node);
        let stop = entry.map_or(lines.lines.len(), |(_, line)| line);
        if stop > start || !blocks.is_empty() {
            // The margins below a spanner reach the lines that follow it. The
            // line engine keeps the positions of lines on a 1/64 px grid.
            y = ((y + pending.resolve()) * 64.0).round() / 64.0;
            pending = CollapsibleMarginSet::ZERO;
            // A group above a spanner is balanced even under `column-fill:
            // auto`, which fills the columns of the last one. In a container
            // whose height is bounded, each group has what the groups and
            // spanners above it leave of that height (CSS Multi-column 1,
            // 7.1 and 8.1), and at least a pixel, as in Chromium: each of
            // its columns then holds a line.
            let group = FragmentationContext {
                column_fill: if spanner.is_some() {
                    ColumnFillValue::Balance
                } else {
                    context.column_fill
                },
                available_height: context.available_height.map(|height| (height - y).max(1.0)),
                ..context
            };
            let (mut columns, height, occupied, baseline) = if blocks.is_empty() {
                let (columns, height) = group_column_fragments(lines, start..stop, group, y);
                let occupied = columns.iter().map(|column| column.fragmentainer).collect();
                let baseline = (stop > start)
                    .then(|| line_baseline(lines, &columns, start))
                    .flatten();
                (columns, height, occupied, baseline)
            } else {
                let units = units_of(lines, start..stop, &blocks);
                let placed = place_units(&units, group, y);
                if perform {
                    move_blocks(tree, &units, &placed, group);
                }
                let baseline = units_first_baseline(lines, &units, &placed, &blocks);
                let occupied = placed.columns.iter().map(|&(column, _)| column).collect();
                (placed.fragments, placed.height, occupied, baseline)
            };
            blocks.clear();
            if perform {
                tree.nodes[owner].multicol_groups.push(MulticolGroup {
                    // Balanced columns are as tall as the group; those that
                    // fill in turn reach the height the group has.
                    context: FragmentationContext {
                        origin_x: geometry.edges.0,
                        origin_y: geometry.top_edge + y,
                        available_height: group
                            .available_height
                            .filter(|_| group.column_fill == ColumnFillValue::Auto),
                        ..group
                    },
                    height,
                    occupied,
                    slice: None,
                    run_start: None,
                    min_height: 0.0,
                });
            }
            first_baseline = first_baseline.or(baseline);
            if stop > start {
                last_baseline = line_baseline(lines, &columns, stop - 1);
            }
            fragments.append(&mut columns);
            y += height;
        }
        if let Some(node) = spanner {
            if perform {
                spanning::clear_column_state(tree, node);
            }
            let placed = place_block_across(tree, node, y, pending, across, perform);
            first_baseline = first_baseline.or(placed.first_baseline);
            (y, pending) = (placed.next_y, placed.pending);
        }
        start = stop;
    }
    Groups {
        fragments,
        height: y + pending.resolve(),
        first_baseline,
        last_baseline,
    }
}

/// What the columns of a group place, in the order of the paragraph: a line,
/// or a block between its lines.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    Line(usize),
    Block(usize),
}

/// The flow extent and first baseline the line layout of `lines` gave the
/// block `node`, the `index`th block of the paragraph.
fn block_box(lines: &IfcLines, index: usize, node: usize) -> (f32, f32, Option<f32>) {
    lines
        .block_boxes
        .get(index)
        .filter(|entry| entry.0 == node)
        .or_else(|| lines.block_boxes.iter().find(|entry| entry.0 == node))
        .map_or((0.0, 0.0, None), |&(_, top, bottom, baseline)| {
            (top, bottom, baseline)
        })
}

/// The units of the group of `lines` in `range` with the `blocks` among them,
/// each with its block-start and block-end offsets from the content box. A
/// block comes with the index of the first line after it and its index among
/// the blocks of the paragraph.
fn units_of(
    lines: &IfcLines,
    range: std::ops::Range<usize>,
    blocks: &[(usize, usize, usize)],
) -> Vec<(Unit, f32, f32)> {
    let mut units = Vec::with_capacity(range.len() + blocks.len());
    let mut pending = blocks.iter().peekable();
    let mut push_blocks_before = |line: usize, units: &mut Vec<(Unit, f32, f32)>| {
        while let Some(&(node, _, index)) = pending.next_if(|&&(_, after, _)| after <= line) {
            let (top, bottom, _) = block_box(lines, index, node);
            units.push((Unit::Block(node), top, bottom));
        }
    };
    for index in range.clone() {
        push_blocks_before(index, &mut units);
        let line = &lines.lines[index];
        units.push((
            Unit::Line(index),
            line.block_offset(),
            line.block_offset() + line.block_size(),
        ));
    }
    push_blocks_before(usize::MAX, &mut units);
    units
}

/// The first baseline of `units` once `placed` in columns, from the top of
/// the content box: the highest of the first baselines of the columns, each
/// that of its first line or block among `blocks` that has one, as in
/// Chromium.
fn units_first_baseline(
    lines: &IfcLines,
    units: &[(Unit, f32, f32)],
    placed: &PlacedUnits,
    blocks: &[(usize, usize, usize)],
) -> Option<f32> {
    let mut done = std::collections::BTreeSet::new();
    let mut highest: Option<f32> = None;
    for (&(unit, _, _), &(column, dy)) in units.iter().zip(&placed.columns) {
        if done.contains(&column) {
            continue;
        }
        let baseline = match unit {
            Unit::Line(line) => line_baseline(lines, &placed.fragments, line),
            Unit::Block(node) => blocks
                .iter()
                .find(|entry| entry.0 == node)
                .and_then(|&(_, _, index)| block_box(lines, index, node).2)
                .map(|baseline| baseline + dy),
        };
        if let Some(baseline) = baseline {
            done.insert(column);
            highest = Some(highest.map_or(baseline, |highest| highest.min(baseline)));
        }
    }
    highest
}

/// The columns of a group of units.
struct PlacedUnits {
    /// The lines of each column, one fragment for each run of lines that no
    /// block interrupts.
    fragments: Vec<MulticolTextFragment>,
    /// Each unit's column, and the offset that takes it there along the
    /// block axis.
    columns: Vec<(usize, f32)>,
    height: f32,
}

/// Place `units` in the columns of `context` from the block offset `y`: in
/// turn, each column as tall as the definite height of a group that fills
/// them, or as short as lets the group fit its columns when it balances. A
/// unit is not broken: a line or block that does not fit what is left of a
/// column starts the next one, and `orphans` and `widows` hold for each run
/// of lines between the blocks, as for the lines of an anonymous block.
fn place_units(units: &[(Unit, f32, f32)], context: FragmentationContext, y: f32) -> PlacedUnits {
    let available = context
        .column_count
        .saturating_sub(context.column_index)
        .max(1);
    let height = column_height(units, context, available);
    let starts = column_starts(units, height, context);
    let mut fragments: Vec<MulticolTextFragment> = Vec::new();
    let mut columns = Vec::with_capacity(units.len());
    let mut tallest = 0.0f32;
    for (column, &start) in starts.iter().enumerate() {
        let end = starts.get(column + 1).copied().unwrap_or(units.len());
        let column = context.column_index + column;
        let origin = units[start].1;
        for (index, &(unit, top, bottom)) in units.iter().enumerate().take(end).skip(start) {
            tallest = tallest.max(bottom - origin);
            columns.push((column, y - origin));
            let Unit::Line(line) = unit else {
                continue;
            };
            match fragments.last_mut() {
                Some(last)
                    if last.fragmentainer == column
                        && last.line_end == line
                        && index > start
                        && matches!(units[index - 1].0, Unit::Line(_)) =>
                {
                    last.line_end = line + 1;
                }
                _ => fragments.push(MulticolTextFragment {
                    line_start: line,
                    line_end: line + 1,
                    fragmentainer: column,
                    x: context.column_offset_x(column),
                    y: y + top - origin,
                }),
            }
        }
    }
    PlacedUnits {
        fragments,
        columns,
        height: tallest,
    }
}

/// The height of the columns of `units`: the definite height of columns that
/// fill in turn, or, for columns that balance, the least height at which the
/// units fit the `available` columns, no taller than a definite height. Fill
/// with no height puts every unit in the first column.
fn column_height(
    units: &[(Unit, f32, f32)],
    context: FragmentationContext,
    available: usize,
) -> f32 {
    let cap = context.available_height.filter(|height| *height > 0.0);
    if context.column_fill == ColumnFillValue::Auto {
        return cap.unwrap_or(f32::INFINITY);
    }
    let total = units.last().map_or(0.0, |unit| unit.2) - units.first().map_or(0.0, |unit| unit.1);
    let fits = |height: f32| column_starts(units, height, context).len() <= available;
    let mut high = cap.map_or(total, |cap| cap.min(total));
    // Units that overflow the columns at the cap leave nothing to search.
    let tallest = units
        .iter()
        .map(|&(_, top, bottom)| bottom - top)
        .fold(0.0f32, f32::max);
    let mut low = if fits(high) { tallest.min(high) } else { high };
    for _ in 0..32 {
        if high - low <= 1.0 / 64.0 {
            break;
        }
        let middle = (low + high) * 0.5;
        if fits(middle) {
            high = middle;
        } else {
            low = middle;
        }
    }
    high
}

/// The index of the first unit of each column `height` tall that `units`
/// fill in turn. A unit taller than a column takes one of its own.
fn column_starts(
    units: &[(Unit, f32, f32)],
    height: f32,
    context: FragmentationContext,
) -> Vec<usize> {
    let mut starts = vec![0];
    let mut origin = units.first().map_or(0.0, |unit| unit.1);
    let mut index = 1;
    while index < units.len() {
        if units[index].2 - origin <= height + f32::EPSILON {
            index += 1;
            continue;
        }
        let column_start = starts[starts.len() - 1];
        let start = keep_lines_together(units, index, column_start, context)
            .filter(|&start| start > column_start);
        let start = start.unwrap_or(index);
        starts.push(start);
        origin = units[start].1;
        index = start + 1;
    }
    starts
}

/// Where a break before `units[index]` moves to keep `orphans` lines before
/// it in the column that starts at `column_start`, and `widows` after it, in
/// the run of lines it falls in: earlier in the run, or before the run when
/// the run cannot keep both. `None` when the break does not fall between two
/// lines.
fn keep_lines_together(
    units: &[(Unit, f32, f32)],
    index: usize,
    column_start: usize,
    context: FragmentationContext,
) -> Option<usize> {
    let is_line = |at: usize| matches!(units.get(at), Some((Unit::Line(_), _, _)));
    if index == 0 || !is_line(index) || !is_line(index - 1) {
        return None;
    }
    let run_start = (0..index).rev().take_while(|&at| is_line(at)).last()?;
    let run_end = (index..units.len()).take_while(|&at| is_line(at)).last()? + 1;
    let mut start = index;
    let after = run_end - start;
    if after < context.widows {
        start = start.saturating_sub(context.widows - after).max(run_start);
    }
    if start.saturating_sub(run_start.max(column_start)) < context.orphans {
        start = run_start;
    }
    Some(start)
}

/// Move the blocks among `units` to the columns `placed` gives them.
fn move_blocks(
    tree: &mut LayoutSession<'_>,
    units: &[(Unit, f32, f32)],
    placed: &PlacedUnits,
    context: FragmentationContext,
) {
    for (&(unit, _, _), &(column, dy)) in units.iter().zip(&placed.columns) {
        if let Unit::Block(node) = unit {
            let mut layout = tree.nodes[node].layout;
            layout.location.x += context.column_offset_x(column);
            layout.location.y += dy;
            tree.set_unrounded_layout(NodeId::from(node), &layout);
        }
    }
}
