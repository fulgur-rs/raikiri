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
//! column with the lines, and go with them to the columns of their group. A
//! column end cuts a block with nothing in it but its own block size, and the
//! rest of it goes on in the next column; any other block moves whole.

use super::*;
use crate::fragment::MulticolGroup;
use crate::layout::ifc::boxes::{IfcBoxKind, place_block_across};
use crate::layout::ifc::flow::FlowGeometry;
use crate::layout::ifc::root::{BlockBox, IfcLines, IfcRoot};
use crate::layout::protocol::CollapsibleMarginSet;

/// Whether the paragraph `root` of the multicol container `idx` is laid out in
/// groups of columns around its blocks: it holds at least one block, and no
/// float or out-of-flow box, which would need a place in the columns of its
/// own. A block that is not a spanner is cut only if it is empty, and
/// otherwise moves whole to a column, so it may neither force a break in the
/// columns, nor hold a float that the lines after it would flow around where
/// it was first laid out.
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
                let (units, end) = units_of(tree, lines, start..stop, &blocks);
                let placed = place_units(&units, end, group, y);
                if perform {
                    move_blocks(tree, &units, &placed, group);
                }
                let baseline = units_first_baseline(lines, &units, &placed, &blocks);
                let occupied = placed.occupied().into_iter().collect();
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
    /// A block with nothing in it but its own block size, which the column
    /// ends cut, as CSS Fragmentation 3, 4.4 allows: the rest of it goes on
    /// at the top of the next column.
    Leaf(usize),
}

impl Unit {
    fn block(self) -> Option<usize> {
        match self {
            Unit::Line(_) => None,
            Unit::Block(node) | Unit::Leaf(node) => Some(node),
        }
    }
}

/// Where the line layout of `lines` put the block `node`, the `index`th
/// block of the paragraph.
fn block_box(lines: &IfcLines, index: usize, node: usize) -> BlockBox {
    lines
        .block_boxes
        .get(index)
        .filter(|entry| entry.node == node)
        .or_else(|| lines.block_boxes.iter().find(|entry| entry.node == node))
        .copied()
        .unwrap_or(BlockBox {
            node,
            top: 0.0,
            bottom: 0.0,
            end: 0.0,
            first_baseline: None,
            start: 0.0,
        })
}

/// The units of the group of `lines` in `range` with the `blocks` among them,
/// each with its block-start and block-end offsets from the content box. A
/// block comes with the index of the first line after it and its index among
/// the blocks of the paragraph. A block that starts the group keeps the
/// margins above it in the first column, which no break precedes, so its
/// unit starts where the content before it ends. Also the offset where the content of the group ends,
/// below the margins of a block that ends it.
fn units_of(
    tree: &LayoutSession<'_>,
    lines: &IfcLines,
    range: std::ops::Range<usize>,
    blocks: &[(usize, usize, usize)],
) -> (Vec<(Unit, f32, f32)>, f32) {
    let mut units = Vec::with_capacity(range.len() + blocks.len());
    let mut end = 0.0;
    let mut pending = blocks.iter().peekable();
    let mut push_blocks_before = |line: usize, units: &mut Vec<(Unit, f32, f32)>, end: &mut f32| {
        while let Some(&(node, _, index)) = pending.next_if(|&&(_, after, _)| after <= line) {
            let block = block_box(lines, index, node);
            let top = if units.is_empty() {
                block.start
            } else {
                block.top
            };
            let unit = if lineless::slices_among_lines(tree, node) {
                Unit::Leaf(node)
            } else {
                Unit::Block(node)
            };
            units.push((unit, top, block.bottom));
            *end = block.end;
        }
    };
    for index in range.clone() {
        push_blocks_before(index, &mut units, &mut end);
        let line = &lines.lines[index];
        end = line.block_offset() + line.block_size();
        units.push((Unit::Line(index), line.block_offset(), end));
    }
    push_blocks_before(usize::MAX, &mut units, &mut end);
    (units, end)
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
    let indices: std::collections::HashMap<usize, usize> = blocks
        .iter()
        .map(|&(node, _, index)| (node, index))
        .collect();
    let mut done = std::collections::BTreeSet::new();
    let mut highest: Option<f32> = None;
    for (&(unit, _, _), &(column, dy)) in units.iter().zip(&placed.columns) {
        if done.contains(&column) {
            continue;
        }
        // `dy` takes a unit to its column, so neither lookup scans the
        // fragments or blocks of the group.
        let baseline = match unit {
            Unit::Line(line) => lines.lines.get(line).map(|line| {
                line.block_offset() + dy + line.baseline(shodo::geometry::BaselineKind::Alphabetic)
            }),
            Unit::Block(node) | Unit::Leaf(node) => indices
                .get(&node)
                .and_then(|&index| block_box(lines, index, node).first_baseline)
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
    /// block axis: for a block that the column ends cut, those of its first
    /// piece.
    columns: Vec<(usize, f32)>,
    /// For each block that the column ends cut, the index of its unit and
    /// its pieces: the column of each, and its block offset and size in the
    /// content box.
    pieces: Vec<(usize, Vec<Piece>)>,
    height: f32,
}

/// A piece of a block that the column ends cut: its column, and its block
/// offset and size in the content box.
type Piece = (usize, f32, f32);

impl PlacedUnits {
    /// The columns that hold a unit or a piece of one.
    fn occupied(&self) -> Vec<usize> {
        let mut columns: Vec<usize> = self.columns.iter().map(|&(column, _)| column).collect();
        for (_, pieces) in &self.pieces {
            columns.extend(pieces.iter().map(|&(column, ..)| column));
        }
        columns
    }
}

/// Where a column starts among the units of a group: the unit it starts
/// with, and the offset of its top in the flow of the group. A column that
/// starts inside a block the column end above it cut `continues` that block.
#[derive(Clone, Copy, Debug)]
struct ColumnStart {
    unit: usize,
    origin: f32,
    continues: bool,
}

/// How far past the end of a column the content in it may reach and still
/// fit: well under the 1/64 px grid of Chromium's layout units, to absorb the
/// rounding of a height that the content divides evenly.
const FIT_TOLERANCE: f32 = 1.0e-3;

/// Place `units` in the columns of `context` from the block offset `y`: in
/// turn, each column as tall as the definite height of a group that fills
/// them, or as short as lets the group fit its columns when it balances. A
/// line or block that does not fit what is left of a column starts the next
/// one, and `orphans` and `widows` hold for each run of lines between the
/// blocks, as for the lines of an anonymous block. A block with nothing in it
/// is cut at the end of the column instead, and goes on in the next.
fn place_units(
    units: &[(Unit, f32, f32)],
    end: f32,
    context: FragmentationContext,
    y: f32,
) -> PlacedUnits {
    let available = context
        .column_count
        .saturating_sub(context.column_index)
        .max(1);
    let height = column_height(units, end, context, available);
    let starts = column_starts(units, height, context);
    let mut fragments: Vec<MulticolTextFragment> = Vec::new();
    let mut columns = Vec::with_capacity(units.len());
    let mut pieces: Vec<(usize, Vec<Piece>)> = Vec::new();
    let mut tallest = 0.0f32;
    for (column, start) in starts.iter().enumerate() {
        let next = starts.get(column + 1);
        let stop = next.map_or(units.len(), |next| next.unit + usize::from(next.continues));
        let column = context.column_index + column;
        let origin = start.origin;
        for (index, &(unit, top, bottom)) in units.iter().enumerate().take(stop).skip(start.unit) {
            let continued = index == start.unit && start.continues;
            let cut = next.filter(|next| next.continues && next.unit == index);
            let piece_top = if continued { origin } else { top };
            let piece_bottom = cut.map_or(bottom, |next| next.origin);
            tallest = tallest.max(piece_bottom - origin);
            if !continued {
                columns.push((column, y - origin));
            }
            if continued || cut.is_some() {
                let piece = (column, y + piece_top - origin, piece_bottom - piece_top);
                match pieces.last_mut() {
                    Some((last, list)) if *last == index => list.push(piece),
                    _ => pieces.push((index, vec![piece])),
                }
            }
            let Unit::Line(line) = unit else {
                continue;
            };
            match fragments.last_mut() {
                Some(last)
                    if last.fragmentainer == column
                        && last.line_end == line
                        && index > start.unit
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
    // The margins below a block that ends the group reach the end of its
    // last column at most.
    if let Some(last) = starts.last() {
        tallest = tallest.max((end - last.origin).min(height));
    }
    // Balanced columns are as tall as the height they balance at.
    if context.column_fill == ColumnFillValue::Balance && height.is_finite() {
        tallest = tallest.max(height);
    }
    PlacedUnits {
        fragments,
        columns,
        pieces,
        height: tallest,
    }
}

/// The height of the columns of `units`, whose content ends at `end`: the
/// definite height of columns that fill in turn, or, for columns that
/// balance, the least height at which the units fit the `available` columns,
/// no taller than a definite height. As in Chromium, balanced columns are at
/// least as tall as an even share of the content, margins below its last
/// block included. Fill with no height puts every unit in the first column.
fn column_height(
    units: &[(Unit, f32, f32)],
    end: f32,
    context: FragmentationContext,
    available: usize,
) -> f32 {
    let cap = context.available_height.filter(|height| *height > 0.0);
    if context.column_fill == ColumnFillValue::Auto {
        return cap.unwrap_or(f32::INFINITY);
    }
    let total = end - units.first().map_or(0.0, |unit| unit.1);
    let fits = |height: f32| column_starts(units, height, context).len() <= available;
    let mut high = cap.map_or(total, |cap| cap.min(total));
    // Units that overflow the columns at the cap leave nothing to search. A
    // block that the column ends cut needs no column as tall as itself.
    let tallest = units
        .iter()
        .filter(|unit| !matches!(unit.0, Unit::Leaf(_)))
        .map(|&(_, top, bottom)| bottom - top)
        .fold(0.0f32, f32::max);
    // Chromium lays out on a grid of 1/64 px, and rounds the share up to it.
    let share = (total / available as f32 * 64.0 - FIT_TOLERANCE).ceil() / 64.0;
    let mut low = if fits(high) {
        tallest.max(share).min(high)
    } else {
        high
    };
    if fits(low) {
        return low;
    }
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
    // The search stops within 1/64 px above the height the content of the
    // tallest column needs, which is the height it settles on.
    let starts = column_starts(units, high, context);
    let needed = starts
        .iter()
        .enumerate()
        .map(|(column, start)| match starts.get(column + 1) {
            Some(next) if next.continues => next.origin - start.origin,
            next => units[next.map_or(units.len(), |next| next.unit) - 1].2 - start.origin,
        })
        .fold(0.0f32, f32::max);
    needed.clamp(share.min(high), high)
}

/// Where each column `height` tall that `units` fill in turn starts. A unit
/// taller than a column takes one of its own, unless it is a block that the
/// column ends cut: each column then ends inside it, and the next goes on
/// with the rest of it, up to [`paragraph_group::MAX_SLICED_PIECES`] columns.
fn column_starts(
    units: &[(Unit, f32, f32)],
    height: f32,
    context: FragmentationContext,
) -> Vec<ColumnStart> {
    let runs = line_runs(units);
    let mut starts = vec![ColumnStart {
        unit: 0,
        origin: units.first().map_or(0.0, |unit| unit.1),
        continues: false,
    }];
    let mut index = 0;
    while index < units.len() {
        let current = starts[starts.len() - 1];
        let (unit, top, bottom) = units[index];
        if bottom - current.origin <= height + FIT_TOLERANCE {
            index += 1;
            continue;
        }
        let cut = current.origin + height;
        // A cut needs a little of the block on either side of it.
        if matches!(unit, Unit::Leaf(_))
            && cut - top >= paragraph_group::SLICE_MIN_ROOM
            && bottom - cut > paragraph_group::SLICE_MIN_ROOM
            && starts.len() < paragraph_group::MAX_SLICED_PIECES
        {
            starts.push(ColumnStart {
                unit: index,
                origin: cut,
                continues: true,
            });
            continue;
        }
        if index == current.unit {
            index += 1;
            continue;
        }
        let start = keep_lines_together(&runs, index, current.unit, context)
            .filter(|&start| start > current.unit)
            .unwrap_or(index);
        starts.push(ColumnStart {
            unit: start,
            origin: units[start].1,
            continues: false,
        });
        index = start;
    }
    starts
}

/// For each of `units`, the run of lines it falls in, as the indices of its
/// first unit and of the unit after it; `None` for a block.
fn line_runs(units: &[(Unit, f32, f32)]) -> Vec<Option<(usize, usize)>> {
    let mut runs = vec![None; units.len()];
    let mut index = 0;
    while index < units.len() {
        let end = (index..units.len())
            .find(|&at| !matches!(units[at].0, Unit::Line(_)))
            .unwrap_or(units.len());
        runs[index..end].fill(Some((index, end)));
        index = end + 1;
    }
    runs
}

/// Where a break before the unit `index` moves to keep `orphans` lines before
/// it in the column that starts at `column_start`, and `widows` after it, in
/// the run of lines it falls in, as `runs` gives them: earlier in the run, or
/// before the run when the run cannot keep both. `None` when the break does
/// not fall between two lines.
fn keep_lines_together(
    runs: &[Option<(usize, usize)>],
    index: usize,
    column_start: usize,
    context: FragmentationContext,
) -> Option<usize> {
    let (run_start, run_end) = runs.get(index).copied().flatten()?;
    if run_start == index {
        return None;
    }
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

/// Move the blocks among `units` to the columns `placed` gives them, and
/// record a fragment for each piece of a block that the column ends cut.
fn move_blocks(
    tree: &mut LayoutSession<'_>,
    units: &[(Unit, f32, f32)],
    placed: &PlacedUnits,
    context: FragmentationContext,
) {
    for (&(unit, _, _), &(column, dy)) in units.iter().zip(&placed.columns) {
        if let Some(node) = unit.block() {
            let mut layout = tree.nodes[node].layout;
            layout.location.x += context.column_offset_x(column);
            layout.location.y += dy;
            tree.set_unrounded_layout(NodeId::from(node), &layout);
            // The pieces of an earlier layout no longer apply.
            tree.fragment_tree.retire_previous_roots(node);
        }
    }
    for (index, pieces) in &placed.pieces {
        if let Some(node) = units[*index].0.block() {
            record_pieces(tree, node, pieces, context);
        }
    }
}

/// Record a fragment for each of the `pieces` of the block `node`, which
/// stands where the first one is: each piece is as tall as the part of the
/// block its column holds. A block that slices its borders and padding
/// paints whole in each piece instead, moved up by what the pieces before it
/// took, and the column clips it.
fn record_pieces(
    tree: &mut LayoutSession<'_>,
    node: usize,
    pieces: &[Piece],
    context: FragmentationContext,
) {
    let Some(&(first_column, first_y, _)) = pieces.first() else {
        return;
    };
    let layout = tree.nodes[node].layout;
    let sliced = lineless::slices_decorations(tree, node);
    let mut taken = 0.0f32;
    for (index, &(column, top, height)) in pieces.iter().enumerate() {
        let rect = crate::fragment::FragmentRect {
            x: context.column_offset_x(column) - context.column_offset_x(first_column),
            y: top - first_y,
            width: layout.size.width,
            height,
        };
        let fragment = crate::fragment::LayoutFragment {
            node_id: node,
            parent: None,
            fragmentainer: column,
            rect: lineless::sliced_rect(rect, sliced.then_some((taken, layout.size.height))),
            // Only a piece that paints the whole box needs the column to cut
            // it; the others are as tall as their part of it.
            fragmentainer_clip: sliced.then(|| paragraph_group::inline_open_clip(rect)),
            fragment_index: index,
            fragment_count: pieces.len(),
            line_start: None,
            line_end: None,
        };
        if tree.fragment_tree.try_push(fragment).is_none() {
            // Without all its pieces, the block paints whole where it stands.
            tree.fragment_tree.retire_previous_roots(node);
            return;
        }
        taken += height;
    }
}
