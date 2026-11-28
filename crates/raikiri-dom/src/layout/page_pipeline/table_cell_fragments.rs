//! A single-line cell can keep its text while its spare height continues,
//! a row breaks across the page end unless a cell of it breaks before its
//! content, and a cell that a page break splits drops its block-axis
//! alignment.

use super::*;
use raikiri_layout_style::values::ResolveOrZero;
use raikiri_style::property::{BreakInside, VerticalAlign};

#[cfg(test)]
mod tests;

pub(super) struct CellFragment {
    cell: usize,
    shift: f32,
}

impl CellFragment {
    /// Retain the cell's height, moving only its alignment room below the
    /// content: the padding that carried it and the children in flow that the
    /// table moved by it.
    pub(super) fn apply(
        self,
        document: &mut super::super::context::LayoutSession<'_>,
    ) -> (usize, f32) {
        let mut layout = document.nodes[self.cell].layout;
        layout.padding.top -= self.shift;
        layout.padding.bottom += self.shift;
        document.store_layout(self.cell, &layout);
        let children = document
            .ifc_layout_node(self.cell)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        for child in children {
            if document.nodes[child].layout_position() != Position::Absolute {
                let mut child_layout = document.nodes[child].layout;
                child_layout.location.y -= self.shift;
                document.store_layout(child, &child_layout);
            }
        }
        (self.cell, self.shift)
    }
}

/// The cells of `row`, placed with its top at `top` in the flow (without the
/// relative offsets that only move where it is painted), that a break at
/// `page_end` goes through and that their block-axis alignment moved down.
/// Chromium lays a cell that fragments out as if it were top-aligned, whether
/// it was aligned to the middle, the bottom or the row's baseline, so the
/// room the alignment put above the content moves below it. A cell that
/// spans rows is split the same way when its rows cross the page end, and so
/// is a cell with a forced break inside it.
pub(super) fn split(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    row: usize,
    top: f32,
    page_start: f32,
    page_end: f32,
) -> Vec<CellFragment> {
    let row_node = &document.nodes[row];
    if document.context.pagination_on_transposed_axes
        || row_node.display() != DisplayValue::TableRow
    {
        return Vec::new();
    }
    row_node
        .children
        .iter()
        .copied()
        .filter(|&cell| {
            let node = &document.nodes[cell];
            node.display() == DisplayValue::TableCell
                && sibling_renders(document, cell)
                && !node.used_writing_mode.is_vertical()
        })
        .filter_map(|cell| {
            let layout = &document.nodes[cell].layout;
            let cell_top = top + layout.location.y;
            let runs_past = starts_on_page_and_runs_past_it(
                cell_top,
                cell_top + layout.size.height,
                page_start,
                page_end,
            );
            // A forced break inside the cell splits it too.
            let starts_on_page = cell_top >= page_start && cell_top < page_end;
            if !(runs_past || starts_on_page && breaks_inside(document, cascade, cell)) {
                return None;
            }
            let padding = document
                .physical_style(cell)
                .padding()
                .resolve_or_zero(Some(layout.size.width));
            let shift = layout.padding.top - padding.top;
            (shift.is_finite() && shift > 0.001).then_some(CellFragment { cell, shift })
        })
        .collect()
}

/// Qualify only a real, one-cell, one-row table with one direct text line,
/// which stays on its page with the cell's spare height continuing (`split`
/// then moves the cell's alignment room below the text). Other table contents
/// keep their existing row-boundary decisions.
pub(super) fn fitting(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    row: usize,
    top: f32,
    page_start: f32,
    page_end: f32,
) -> bool {
    fits(document, cascade, row, top, page_start, page_end).is_some()
}

fn fits(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    row: usize,
    top: f32,
    page_start: f32,
    page_end: f32,
) -> Option<()> {
    let row_node = document.get_node(row)?;
    if document.context.pagination_on_transposed_axes
        || row_node.display() != DisplayValue::TableRow
    {
        return None;
    }
    let cell = only_box(document, row)?;
    if document.nodes[cell].display() != DisplayValue::TableCell
        || document.nodes[cell].has_before_or_after_content
        || !starts_on_page_and_runs_past_it(
            top,
            top + document.nodes[cell].layout.size.height,
            page_start,
            page_end,
        )
        || document.element_attribute(cell, "rowspan").is_some()
        || document.element_attribute(cell, "colspan").is_some()
        || !document.nodes[cell]
            .children
            .iter()
            .all(|&child| document.nodes[child].kind() == NodeKind::Text)
    {
        return None;
    }
    let mut owner = document.parent_of(row)?;
    let mut first = row;
    if document.nodes[owner].display() == DisplayValue::TableRowGroup {
        if only_box(document, owner) != Some(row) {
            return None;
        }
        first = owner;
        owner = document.parent_of(owner)?;
    }
    if document.nodes[owner].display() != DisplayValue::Table || only_box(document, owner)? != first
    {
        return None;
    }
    for id in document.inclusive_ancestors(cell) {
        let node = &document.nodes[id];
        let computed = cascade.computed.get(id)?;
        if !matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative
        ) || *node.position() == PositionValue::Relative
            && (!inset_is_auto(&computed.top) || !inset_is_auto(&computed.bottom))
            || node.multicol.is_some()
            || node.used_writing_mode.is_vertical()
            || node.size_containment.height
            || element_clips_overflow(document, cascade, id)
            || is_floating_box_for_pagination(document, cascade, id)
            || computed.break_inside != BreakInside::Auto
            || !matches!(
                computed.break_before,
                BreakBetween::Auto | BreakBetween::Avoid | BreakBetween::AvoidPage
            )
            || !matches!(
                computed.break_after,
                BreakBetween::Auto | BreakBetween::Avoid | BreakBetween::AvoidPage
            )
        {
            return None;
        }
    }
    let node = document.ifc_layout_node(cell)?;
    let ifc = node.ifc.as_ref()?;
    let lines = ifc.lines.as_ref()?;
    if lines.lines.len() != 1 || lines.fragmentainer_line_ranges.is_some() {
        return None;
    }
    let layout = &document.nodes[cell].layout;
    let padding = document
        .physical_style(cell)
        .padding()
        .resolve_or_zero(Some(layout.size.width));
    let natural =
        lines.height + padding.top + padding.bottom + layout.border.top + layout.border.bottom;
    if !natural.is_finite() || natural <= 0.0 || top + natural > page_end + 0.001 {
        return None;
    }
    let shift = layout.padding.top - padding.top;
    (matches!(
        node.table_vertical_align,
        VerticalAlign::Top | VerticalAlign::Middle | VerticalAlign::Bottom
    ) && shift.is_finite()
        && shift >= 0.0)
        .then_some(())
}

fn only_box(document: &Document, parent: usize) -> Option<usize> {
    let mut boxes = document.nodes[parent]
        .children
        .iter()
        .copied()
        .filter(|&child| sibling_renders(document, child));
    let child = boxes.next()?;
    boxes.next().is_none().then_some(child)
}

/// The cells of `row` beside `grown`, a cell that a forced break grew, aligned
/// again within the part of the row on the page the break leaves, which ends
/// `end` below the top of the row; `None` when the break falls below the row,
/// in a cell that spans the rows after it, and leaves the row whole. Chromium
/// lays such a row out at the height its fragment on that page has, so a cell
/// aligned to the middle or the bottom keeps that alignment within the fragment, and one aligned to the baseline
/// starts at its top. A cell that ends past `end` itself, or that has a forced
/// break of its own, is split and already aligned to its top (see `split`).
pub(super) fn realign_beside(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    cells: &[usize],
    grown: usize,
    end: Option<f32>,
    split_cells: &std::collections::HashSet<usize>,
) -> Vec<CellFragment> {
    // Only a forced break in the cell's own block flow splits the row there.
    if document.context.pagination_on_transposed_axes
        || end.is_none()
        || !breaks_inside(document, cascade, grown)
    {
        return Vec::new();
    }
    let end = end.unwrap_or_default();
    // Where a cell starts below the top of the grown one: the cells of a
    // row sit at its top, and those of a run outside a table at the top of
    // their anonymous row, below the boxes before the table.
    let grown_y = document.nodes[grown].layout.location.y;
    cells
        .iter()
        .copied()
        .filter(|&cell| {
            let node = &document.nodes[cell];
            cell != grown
                && !split_cells.contains(&cell)
                && node.display() == DisplayValue::TableCell
                && sibling_renders(document, cell)
                && !node.used_writing_mode.is_vertical()
                && node.layout.location.y - grown_y + node.layout.size.height <= end + 0.001
                && !breaks_inside(document, cascade, cell)
        })
        .filter_map(|cell| {
            let layout = &document.nodes[cell].layout;
            let top = layout.location.y - grown_y;
            let padding = document
                .physical_style(cell)
                .padding()
                .resolve_or_zero(Some(layout.size.width));
            let shift = layout.padding.top - padding.top;
            let room = shift + layout.padding.bottom - padding.bottom;
            let fragment_room = (room + end - top - layout.size.height).max(0.0);
            let aligned = match document.ifc_layout_node(cell)?.table_vertical_align {
                VerticalAlign::Middle => fragment_room / 2.0,
                VerticalAlign::Bottom => fragment_room,
                _ => 0.0,
            };
            let moved = shift - aligned;
            (moved.is_finite() && moved.abs() > 0.001)
                .then_some(CellFragment { cell, shift: moved })
        })
        .collect()
}

/// Whether a forced page break falls between content inside `cell`: a box in
/// its block flow forces a break before itself after content of the cell, or
/// after itself before more of it. A break at the start or the end of the
/// cell passes to the row and does not split the cell. Only block-level boxes
/// break pages, so a break on an inline box, or inside a box that does not
/// stack its children in block flow, or out of the flow, does not reach it.
/// Generated content of the cell, or of a block it goes through, counts as
/// content on its own side.
fn breaks_inside(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    cell: usize,
) -> bool {
    breaks_between(
        document,
        cascade,
        cell,
        generated_on_side(cascade, cell, true),
        generated_on_side(cascade, cell, false),
    )
}

/// The children of `parent` that are content of its block flow, with the
/// children of `display: contents` boxes in place of them. Floats,
/// positioned boxes and white space that collapses away are not that content.
fn flow_children(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    parent: usize,
    out: &mut Vec<usize>,
) {
    for &child in &document.nodes[parent].children {
        let node = &document.nodes[child];
        // White space that the style preserves still makes a line.
        let renders = if node.kind() == NodeKind::Text {
            node.text_content().is_some_and(|text| {
                !crate::generated_content::is_collapsed_white_space(
                    text,
                    cascade.computed[child].effective_white_space_collapse,
                )
            })
        } else {
            sibling_renders(document, child)
        };
        if !renders
            || matches!(
                node.position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
            || is_floating_box_for_pagination(document, cascade, child)
        {
            continue;
        }
        if node.kind() == NodeKind::Element
            && node.display() == DisplayValue::Contents
            && !node.has_before_or_after_content
        {
            flow_children(document, cascade, child, out);
        } else {
            out.push(child);
        }
    }
}

/// Whether a forced break falls between content among the children of
/// `parent` or inside them, with whether content of the cell comes before and
/// after `parent`.
fn breaks_between(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    parent: usize,
    before: bool,
    after: bool,
) -> bool {
    let mut children = Vec::new();
    flow_children(document, cascade, parent, &mut children);
    children.iter().enumerate().any(|(index, &child)| {
        let node = &document.nodes[child];
        if node.kind() != NodeKind::Element || !is_block_level_flow_box(node.display()) {
            return false;
        }
        let computed = &cascade.computed[child];
        let before = before || index > 0;
        let after = after || index + 1 < children.len();
        page_break_is_forced(document, cascade, child, computed.break_before) && before
            || page_break_is_forced(document, cascade, child, computed.break_after) && after
            || (breaks_through_block_flow(document, cascade, child)
                || stacks_children_in_block_flow(document, child)
                    && !is_monolithic_for_pagination_with_transposed_axes(
                        document, cascade, child, false,
                    ))
                && breaks_between(
                    document,
                    cascade,
                    child,
                    before || generated_on_side(cascade, child, true),
                    after || generated_on_side(cascade, child, false),
                )
    })
}

/// Whether a break at `page_end` splits `row`, placed with its top at `top`
/// in the flow, rather than moving it whole to the next page. Chromium moves
/// a row that crosses the page end only when a cell of it breaks before any
/// of its content, as when its first line, or a first box that no break goes
/// through, runs past the page end, or too few lines fit for its orphans.
/// Otherwise the row breaks there with its cells, each laid out as if it
/// were aligned to its top (see `split`), and their content goes on at the
/// top of the next page.
///
/// The row is split only where its content can break at the page end: no
/// replaced box runs past it (a box that avoids a break inside it moves down
/// inside the cell, as in a taller row), lines that cross it or that its
/// orphans or widows pull below it can move down in their paragraph, which
/// then grows the row (see [`super::cell_flow_inline`]), and no edges that
/// close a box are left without content to go with them. Rows whose lines
/// cannot move that way, and content this cannot follow (boxes laid out side
/// by side, floats across the page end that do not move down inside the
/// cell, see [`float_crosses`], lines around a block), still move whole.
pub(super) fn row_splits(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    row: usize,
    top: f32,
    page_end: f32,
) -> bool {
    let cells: Vec<usize> = document.nodes[row]
        .children
        .iter()
        .copied()
        .filter(|&cell| sibling_renders(document, cell))
        .collect();
    let row_top = cells
        .iter()
        .map(|&cell| document.pagination_local_y(cell))
        .fold(f32::INFINITY, f32::min);
    // The cells of earlier rows that span the row break with it where they
    // are: their content before it is already on the page.
    let spanning = document
        .ancestors(row)
        .find(|&id| {
            matches!(
                document.nodes[id].display(),
                DisplayValue::Table | DisplayValue::InlineTable
            )
        })
        .map_or_else(Vec::new, |table| {
            crate::layout::table::row_moves::spanning_cells_beside(document, table, row)
        });
    let row_abs_y = current_abs_y(document, row);
    !document.context.pagination_on_transposed_axes
        && !cells.is_empty()
        && cells.iter().all(|&cell| {
            let cell_top = top + document.pagination_local_y(cell) - row_top;
            cell_breaks(document, cascade, cell, cell_top, page_end, Some(body_id))
        })
        && spanning.iter().all(|&cell| {
            let cell_top = top + current_abs_y(document, cell) - row_abs_y;
            cell_breaks(document, cascade, cell, cell_top, page_end, None)
        })
}

/// Whether the break at `page_end` that splits `row`, placed with its top at
/// `top` in the flow, leaves lines of its cells to go on below a copy of a
/// repeated header on the next page: every cell either ends its content
/// before the break or lays it out in lines that move down inside it (see
/// [`super::cell_flow_inline`]), and some cell has lines past the break.
/// Those moves carry the copies of the table's repeated groups (see
/// `HeaderRepeats::place_copies_around_row_break`); a row that breaks
/// without moving lines still moves whole in such a table.
pub(super) fn lines_carry_row_break(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    row: usize,
    top: f32,
    page_end: f32,
) -> bool {
    let cells: Vec<usize> = document.nodes[row]
        .children
        .iter()
        .copied()
        .filter(|&cell| sibling_renders(document, cell))
        .collect();
    let row_top = cells
        .iter()
        .map(|&cell| document.pagination_local_y(cell))
        .fold(f32::INFINITY, f32::min);
    let spanned = document
        .ancestors(row)
        .find(|&id| {
            matches!(
                document.nodes[id].display(),
                DisplayValue::Table | DisplayValue::InlineTable
            )
        })
        .is_some_and(|table| {
            !crate::layout::table::row_moves::spanning_cells_beside(document, table, row).is_empty()
        });
    let mut carried = false;
    let all_follow = cells.iter().all(|&cell| {
        let layout = &document.nodes[cell].layout;
        let padding = document
            .physical_style(cell)
            .padding()
            .resolve_or_zero(Some(layout.size.width));
        let content_top =
            top + document.pagination_local_y(cell) - row_top + layout.border.top + padding.top;
        let Some(bounds) = line_bounds(document, cell) else {
            return false;
        };
        let past = bounds
            .last()
            .is_some_and(|&(_, bottom)| content_top + bottom > page_end + 0.001);
        carried |= past;
        !past || lines_move_in_cell(document, cascade, cell, body_id)
    });
    !spanned && all_follow && carried
}

/// How far below the top of the row `row` the first break that splits it
/// can go (see [`row_splits`]): the end of the first content of its cells
/// that a page can hold apart from the rest of the row, when a break at any
/// later end of that content splits the row too, so that the page end can
/// fall anywhere past it. `None` when no such break exists, and when the row
/// avoids breaks inside it.
pub(super) fn first_split_end(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    row: usize,
) -> Option<f32> {
    let node = &document.nodes[row];
    if document.context.pagination_on_transposed_axes
        || box_avoids_page_break_inside(document, row, cascade.computed[row].break_inside)
    {
        return None;
    }
    let height = node.layout.size.height;
    let row_y = current_abs_y(document, row);
    let mut ends = Vec::new();
    let mut tops = Vec::new();
    for &cell in &node.children {
        let cell_node = &document.nodes[cell];
        if cell_node.display() != DisplayValue::TableCell {
            continue;
        }
        // A cell that breaks loses the room its alignment put above its
        // content, so its content ends that much higher (see `cell_breaks`).
        let padding = document
            .physical_style(cell)
            .padding()
            .resolve_or_zero(Some(cell_node.layout.size.width));
        let shift = cell_node.layout.padding.top - padding.top;
        let origin = row_y + shift.max(0.0);
        if let Some(bounds) = line_bounds(document, cell) {
            let content_top = current_abs_y(document, cell) - origin
                + cell_node.layout.border.top
                + cell_node.layout.padding.top;
            ends.extend(bounds.iter().map(|&(_, bottom)| content_top + bottom));
            tops.extend(bounds.iter().map(|&(top, _)| content_top + top));
        }
        content_ends(document, cell, origin, &mut ends, &mut tops);
    }
    ends.sort_by(f32::total_cmp);
    ends.dedup_by(|a, b| (*a - *b).abs() < 0.001);
    ends.retain(|&end| end > 0.001 && end < height - 0.001);
    // Every place a box or line starts or ends, so that a page end between
    // two of them falls inside the same content as any other page end
    // there.
    let mut points: Vec<f32> = ends.iter().copied().chain(tops).collect();
    points.retain(|&point| point > 0.001 && point < height - 0.001);
    points.sort_by(f32::total_cmp);
    points.dedup_by(|a, b| (*a - *b).abs() < 0.001);
    // Walk back from the bottom of the row while every page end, at a point
    // and inside the gap after it, splits the row; the break must come
    // after the latest place where it does not.
    let mut first = None;
    let mut next = height;
    for &point in points.iter().rev() {
        let gap = (point + next) / 2.0;
        if !row_splits(document, cascade, body_id, row, 0.0, gap)
            || !row_splits(document, cascade, body_id, row, 0.0, point)
        {
            break;
        }
        if ends.iter().any(|&end| (end - point).abs() < 0.001) {
            first = Some(point);
        }
        next = point;
    }
    first
}

/// Pushes where each element inside `parent`, and each line of one that lays
/// out lines, ends into `ends` and starts into `tops`, measured below
/// `origin` in the document.
fn content_ends(
    document: &super::super::context::LayoutSession<'_>,
    parent: usize,
    origin: f32,
    ends: &mut Vec<f32>,
    tops: &mut Vec<f32>,
) {
    for &child in &document.nodes[parent].children {
        let node = &document.nodes[child];
        if node.kind() != NodeKind::Element {
            continue;
        }
        let top = current_abs_y(document, child) - origin;
        tops.push(top);
        ends.push(top + node.layout.size.height);
        if let Some(bounds) = line_bounds(document, child) {
            let content_top = top + node.layout.border.top + node.layout.padding.top;
            ends.extend(bounds.iter().map(|&(_, bottom)| content_top + bottom));
            tops.extend(bounds.iter().map(|&(top, _)| content_top + top));
        }
        content_ends(document, child, origin, ends, tops);
    }
}

/// Whether a cell of the row `row`, placed with its top at `top` in the
/// flow, starts its content before a break at `page_end` but puts none of it
/// there, which
/// Chromium takes as a break before the row: the row then starts the next
/// page whole, even when it is taller than that page.
pub(super) fn row_breaks_before_content(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    row: usize,
    top: f32,
    (page_end, next_page_step): (f32, f32),
) -> bool {
    let cells: Vec<usize> = document.nodes[row]
        .children
        .iter()
        .copied()
        .filter(|&cell| {
            sibling_renders(document, cell)
                && document.nodes[cell].display() == DisplayValue::TableCell
        })
        .collect();
    let row_top = cells
        .iter()
        .map(|&cell| document.pagination_local_y(cell))
        .fold(f32::INFINITY, f32::min);
    !document.context.pagination_on_transposed_axes
        && cells.iter().any(|&cell| {
            let layout = &document.nodes[cell].layout;
            let padding = document
                .physical_style(cell)
                .padding()
                .resolve_or_zero(Some(layout.size.width));
            let cell_top = top + document.pagination_local_y(cell) - row_top;
            let shift = layout.padding.top - padding.top;
            let tops = (cell_top - shift, cell_top + layout.border.top + padding.top);
            let closing = padding.bottom + layout.border.bottom;
            // Edges that run past the page end themselves break there. Only
            // lines laid out in the cell itself are judged here: blocks
            // inside it move their own content down. A first line that no
            // page holds stays where it is.
            !document.nodes[cell].used_writing_mode.is_vertical()
                && shift.is_finite()
                && tops.1 < page_end - 0.001
                && line_bounds(document, cell).is_some_and(|bounds| {
                    bounds.first().is_some_and(|&(line_top, line_bottom)| {
                        tops.1 - tops.0 + line_bottom - line_top <= next_page_step + 0.001
                    })
                })
                && !content_fits(document, cascade, cell, tops, closing, page_end)
        })
}

/// Whether the cell `cell`, with its border box at `cell_top` in the flow,
/// lets a break at `page_end` go through it where its content is. `own`
/// holds the body for a cell of the row itself, which must also put some of
/// its content before the break; only such a cell's lines move down inside
/// it. A cell of an earlier row that spans the row passes `None`.
fn cell_breaks(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    cell: usize,
    cell_top: f32,
    page_end: f32,
    own: Option<usize>,
) -> bool {
    let node = &document.nodes[cell];
    let layout = &node.layout;
    let padding = document
        .physical_style(cell)
        .padding()
        .resolve_or_zero(Some(layout.size.width));
    // The room that the cell's alignment put above its content, which a
    // cell that breaks loses.
    let shift = layout.padding.top - padding.top;
    let tops = (cell_top - shift, cell_top + layout.border.top + padding.top);
    let closing = padding.bottom + layout.border.bottom;
    let computed = &cascade.computed[cell];
    // Chromium takes a break forced before a cell, or before its first
    // content, as one before its row, which then starts the next page whole.
    node.display() == DisplayValue::TableCell
        && !node.used_writing_mode.is_vertical()
        && !box_avoids_page_break_inside(document, cell, computed.break_inside)
        && shift.is_finite()
        && (own.is_none()
            || !page_break_is_forced(document, cascade, cell, computed.break_before)
                && !starts_with_forced_break(document, cascade, cell)
                && content_fits(document, cascade, cell, tops, closing, page_end))
        && !float_crosses(
            document,
            cascade,
            cell,
            cell,
            tops.0 - current_abs_y(document, cell),
            page_end,
        )
        && breaks_cleanly(document, cascade, cell, tops, closing, page_end, own)
}

/// Whether the first content of `block` in its flow, at any depth, is a box
/// with a break forced before it.
fn starts_with_forced_break(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
) -> bool {
    let mut children = Vec::new();
    flow_children(document, cascade, block, &mut children);
    children.first().is_some_and(|&first| {
        block_level_element(document, first)
            && (page_break_is_forced(
                document,
                cascade,
                first,
                cascade.computed[first].break_before,
            ) || starts_with_forced_break(document, cascade, first))
    })
}

/// Whether a float inside `parent` whose content does not break where it is
/// runs past `page_end`: one that does not stack its content in block flow,
/// or whose lines or boxes do not break cleanly there (see
/// [`float_breaks`]). `offset` takes a box from where it is in the document
/// to where it is in the flow, and `cell` is the cell the floats are in.
///
/// A float that avoids a break inside it crosses only when it cannot move
/// down inside the cell (see [`moves_down`]). A replaced or other box that
/// no break goes through crosses.
fn float_crosses(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    cell: usize,
    parent: usize,
    offset: f32,
    page_end: f32,
) -> bool {
    document.nodes[parent].children.iter().any(|&child| {
        let node = &document.nodes[child];
        node.kind() == NodeKind::Element
            && (is_floating_box_for_pagination(document, cascade, child) && {
                let top = offset + current_abs_y(document, child);
                top < page_end - 0.001
                    && top + node.layout.size.height > page_end + 0.001
                    && (box_is_unbreakable(document, cascade, child, false)
                        || if box_avoids_page_break_inside(
                            document,
                            child,
                            cascade.computed[child].break_inside,
                        ) {
                            !moves_down(document, cascade, cell, (parent, child), offset, page_end)
                        } else {
                            !plain_block(document, child)
                                || !float_breaks(document, cascade, child, top, page_end)
                        })
            } || float_crosses(document, cascade, cell, child, offset, page_end))
    })
}

/// Whether the float `float` in `parent`, which avoids a break inside it and
/// crosses `page_end`, moves down to the next page inside the cell `cell`, as
/// a block that avoids a break does, and the row grows with it: Chromium
/// splits the row there, and keeps the content above the float and the cells
/// beside it on the page. `offset` is as in [`float_crosses`].
///
/// Pagination does not lay out again what goes beside the float or below it
/// once it moves, so the float stays where it is when that content depends
/// on it: the lines of a paragraph it sits in, a later float that stays on
/// the page (no float may start above an earlier one, CSS 2.1 §9.5.1), a box
/// with `clear` (which pagination does not place again below the moved
/// float, whichever side it clears), lines beside it after it that run past
/// the page end, or an independent formatting context after it that does.
fn moves_down(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    cell: usize,
    (parent, float): (usize, usize),
    offset: f32,
    page_end: f32,
) -> bool {
    let float_bottom =
        offset + current_abs_y(document, float) + document.nodes[float].layout.size.height;
    let past_page_end =
        |id: usize, bottom: f32| offset + current_abs_y(document, id) + bottom > page_end + 0.001;
    !super::in_inline_context(document, parent)
        && !super::later_in_float_context(
            document,
            cascade,
            super::float_context_root(document, cascade, float, cell),
            float,
            |id| {
                let node = &document.nodes[id];
                if node.kind() != NodeKind::Element {
                    return false;
                }
                if is_floating_box_for_pagination(document, cascade, id) {
                    let top = offset + current_abs_y(document, id);
                    // A later float that crosses the page end too moves
                    // with it when it can.
                    return top < page_end - 0.001
                        && !(past_page_end(id, node.layout.size.height)
                            && box_avoids_page_break_inside(
                                document,
                                id,
                                cascade.computed[id].break_inside,
                            )
                            && !box_is_unbreakable(document, cascade, id, false));
                }
                let content_top = offset
                    + current_abs_y(document, id)
                    + node.layout.border.top
                    + node.layout.padding.top;
                // A line below the float is there for want of room beside
                // it, and comes back up to where the flow puts it once the
                // float moves on.
                cascade.computed[id].clear != ClearValue::None && !node.display().is_inline_flow()
                    || line_bounds(document, id).is_some_and(|bounds| {
                        bounds.iter().any(|&(line_top, bottom)| {
                            content_top + line_top < float_bottom - 0.001
                                && content_top + bottom > page_end + 0.001
                        })
                    })
                    || super::holds_floats(document, cascade, id)
                        && past_page_end(id, node.layout.size.height)
            },
        )
}

/// Whether the content of the float `float`, with its border box at `top` in
/// the flow, breaks at `page_end` where it is, as the content of a cell has
/// to (see `content_fits` and `breaks_cleanly`): its lines do not move.
fn float_breaks(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    float: usize,
    top: f32,
    page_end: f32,
) -> bool {
    let layout = &document.nodes[float].layout;
    let tops = (top, top + layout.border.top + layout.padding.top);
    let closing = layout.padding.bottom + layout.border.bottom;
    content_fits(document, cascade, float, tops, closing, page_end)
        && breaks_cleanly(document, cascade, float, tops, closing, page_end, None)
}

/// Whether some content of the block `block` goes before a break at
/// `page_end`. `tops` holds where the block's border box and where its
/// content start, and `closing` the padding and border below its content
/// that stay with its last line or box, its own and those of the boxes it
/// ends together with.
fn content_fits(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    (border_top, content_top): (f32, f32),
    closing: f32,
    page_end: f32,
) -> bool {
    if let Some(bounds) = line_bounds(document, block) {
        let count = bounds.len();
        let mut fitting = bounds
            .iter()
            .take_while(|&&(_, bottom)| content_top + bottom <= page_end + 0.001)
            .count();
        // The last line goes to the next page with the edges that close the
        // block when they do not fit after it.
        if fitting == count
            && bounds
                .last()
                .is_some_and(|&(_, bottom)| content_top + bottom + closing > page_end + 0.001)
        {
            fitting -= 1;
        }
        let orphans = cascade.computed[block].orphans.max(1) as usize;
        return count == 0 || fitting >= orphans.min(count) && fitting > 0;
    }
    let mut children = Vec::new();
    flow_children(document, cascade, block, &mut children);
    // A box without content puts its own height before the break.
    children.first().is_none_or(|&first| {
        let layout = &document.nodes[first].layout;
        let child_top = border_top + document.pagination_local_y(first);
        let closing = if children.len() == 1 { closing } else { 0.0 };
        let whole =
            box_avoids_page_break_inside(document, first, cascade.computed[first].break_inside)
                || box_is_unbreakable(document, cascade, first, false);
        block_level_element(document, first)
            && child_top < page_end - 0.001
            && if whole {
                child_top + layout.size.height + closing <= page_end + 0.001
            } else {
                plain_block(document, first)
                    && content_fits(
                        document,
                        cascade,
                        first,
                        (
                            child_top,
                            child_top + layout.border.top + layout.padding.top,
                        ),
                        layout.padding.bottom + layout.border.bottom + closing,
                        page_end,
                    )
            }
    })
}

/// Whether the content of the block `block`, with `tops` and `closing` as in
/// [`content_fits`], breaks at `page_end` where it is, or with only boxes
/// that avoid a break inside them moving down: no line, and no replaced or
/// other box that no break goes through, runs past the page end, the lines
/// of a paragraph on either side of it are enough for its orphans, and for
/// its widows when the paragraph has lines enough for both, and the edges
/// that close a box fit after its content or start past the break with it.
fn breaks_cleanly(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    (border_top, content_top): (f32, f32),
    closing: f32,
    page_end: f32,
    own: Option<usize>,
) -> bool {
    let edges_fit = |content_end: f32| {
        closing <= 0.001
            || content_end > page_end - 0.001
            || content_end + closing <= page_end + 0.001
    };
    if let Some(bounds) = line_bounds(document, block) {
        let computed = &cascade.computed[block];
        let before = bounds
            .iter()
            .filter(|&&(_, bottom)| content_top + bottom <= page_end + 0.001)
            .count();
        let after = bounds
            .iter()
            .filter(|&&(line_top, _)| content_top + line_top >= page_end - 0.001)
            .count();
        let orphans = computed.orphans.max(1) as usize;
        let widows = computed.widows.max(1) as usize;
        // Lines that cross the page end, or that the orphans or widows pull
        // to the next page, have to move down, which only a paragraph whose
        // lines grow the row can do. Lines that cannot move keep the widows
        // only when that leaves enough for the orphans: Chromium otherwise
        // breaks where the lines are and gives up the widows.
        let lines_move =
            own.is_some_and(|body_id| lines_move_in_cell(document, cascade, block, body_id));
        let split = before > 0 && before < bounds.len();
        return (lines_move || before + after == bounds.len())
            && (!split
                || lines_move
                || before >= orphans && (after >= widows || before + after < orphans + widows))
            && bounds
                .last()
                .is_none_or(|&(_, bottom)| edges_fit(content_top + bottom));
    }
    let mut children = Vec::new();
    flow_children(document, cascade, block, &mut children);
    let mut content_end = content_top;
    children.iter().all(|&child| {
        let layout = &document.nodes[child].layout;
        let child_top = border_top + document.pagination_local_y(child);
        let child_end = child_top + layout.size.height;
        let child_content_top = child_top + layout.border.top + layout.padding.top;
        content_end = child_end;
        // A box that avoids a break inside it moves to the next page
        // inside the cell, and the row grows with it.
        block_level_element(document, child)
            && (child_end <= page_end + 0.001
                || child_top >= page_end - 0.001
                || box_avoids_page_break_inside(
                    document,
                    child,
                    cascade.computed[child].break_inside,
                )
                || !box_is_unbreakable(document, cascade, child, false)
                    && plain_block(document, child)
                    && child_content_top <= page_end + 0.001
                    && breaks_cleanly(
                        document,
                        cascade,
                        child,
                        (child_top, child_content_top),
                        layout.padding.bottom + layout.border.bottom,
                        page_end,
                        own,
                    ))
    }) && edges_fit(content_end)
}

/// Whether the lines of the paragraph `block` in a table cell can move down
/// inside it at a page end and grow the cell's row (see
/// [`super::cell_flow_inline`]).
fn lines_move_in_cell(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    block: usize,
    body_id: usize,
) -> bool {
    super::lines_can_shift_inside(document, cascade, block)
        && super::cell_flow_inline(document, cascade, block, body_id).is_some()
}

/// The block-start and block-end edges of the lines of `block`, below the
/// top of its content box, when it lays its content out in lines with no
/// block among them.
fn line_bounds(
    document: &super::super::context::LayoutSession<'_>,
    block: usize,
) -> Option<Vec<(f32, f32)>> {
    let lines = document
        .ifc_layout_node(block)?
        .ifc
        .as_ref()?
        .lines
        .as_ref()?;
    // A block among the lines breaks between boxes, which this does not
    // follow: the lines then count as one that no page holds.
    Some(if lines.block_line_starts.is_empty() {
        ifc_root_line_bounds(document, block).unwrap_or_default()
    } else {
        vec![(0.0, f32::INFINITY)]
    })
}

/// Whether `child` is a block-level element.
fn block_level_element(document: &Document, child: usize) -> bool {
    let node = &document.nodes[child];
    node.kind() == NodeKind::Element && is_block_level_flow_box(node.display())
}

/// Whether `block` stacks its children, or lays out its lines, in plain block
/// flow, in which a break goes between them.
fn plain_block(document: &Document, block: usize) -> bool {
    let node = &document.nodes[block];
    matches!(
        node.display(),
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
    ) && node.multicol.is_none()
        && !node.used_writing_mode.is_vertical()
}
