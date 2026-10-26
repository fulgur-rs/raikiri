//! Column spanners in the paragraph of a multicol container.
//!
//! A multicol container that holds text or inline boxes next to a
//! `column-span: all` block is the root of one paragraph, and the spanner is a
//! block box of it. The lines of the paragraph are broken at the width of a
//! column. A spanner ends the lines above it and starts the lines below it, so
//! the lines between two spanners form a group that is balanced over the
//! columns on its own, and the spanner crosses all of them below the group
//! above (CSS Multi-column Layout 1, 6.1).

use super::*;
use crate::fragment::MulticolGroup;
use crate::layout::ifc::boxes::{IfcBoxKind, place_block_across};
use crate::layout::ifc::flow::FlowGeometry;
use crate::layout::ifc::root::{IfcLines, IfcRoot};
use crate::layout::protocol::CollapsibleMarginSet;

/// Whether the paragraph `root` of the multicol container `idx` is laid out in
/// groups of columns around its spanners: it holds at least one spanner, and
/// no box that would need a place in the columns of its own.
pub(crate) fn supports(tree: &LayoutSession<'_>, idx: usize, root: &IfcRoot) -> bool {
    tree.context.fragmentation_stack.is_empty()
        && tree.nodes[idx]
            .multicol
            .is_some_and(|style| style.horizontal && !style.height_definite)
        && root.generated_blocks.is_empty()
        && root.boxes.iter().any(|box_| box_.kind == IfcBoxKind::Block)
        && root.boxes.iter().all(|box_| match box_.kind {
            IfcBoxKind::Atomic => true,
            IfcBoxKind::Block => spanning::is_spanner(tree, box_.node),
            IfcBoxKind::Float | IfcBoxKind::OutOfFlow => false,
        })
        && !multicol_has_out_of_flow_descendant(tree, idx)
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
    for end in 0..=lines.block_line_starts.len() {
        let spanner = lines.block_line_starts.get(end).map(|&(node, _)| node);
        let stop = lines
            .block_line_starts
            .get(end)
            .map_or(lines.lines.len(), |&(_, line)| line);
        if stop > start {
            // The margins below a spanner reach the lines that follow it. The
            // line engine keeps the positions of lines on a 1/64 px grid.
            y = ((y + pending.resolve()) * 64.0).round() / 64.0;
            pending = CollapsibleMarginSet::ZERO;
            // A group above a spanner is balanced even under `column-fill:
            // auto`, which fills the columns of the last one.
            let group = FragmentationContext {
                column_fill: if spanner.is_some() {
                    ColumnFillValue::Balance
                } else {
                    context.column_fill
                },
                available_height: None,
                ..context
            };
            let (mut columns, height) = group_column_fragments(lines, start..stop, group, y);
            if perform {
                tree.nodes[owner].multicol_groups.push(MulticolGroup {
                    context: FragmentationContext {
                        origin_x: geometry.edges.0,
                        origin_y: geometry.top_edge + y,
                        ..group
                    },
                    height,
                    occupied: columns.iter().map(|column| column.fragmentainer).collect(),
                    slice: None,
                });
            }
            first_baseline = first_baseline.or_else(|| line_baseline(lines, &columns, start));
            last_baseline = line_baseline(lines, &columns, stop - 1);
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
