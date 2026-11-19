//! A single-line cell can keep its text while its spare height continues,
//! and a cell that a page break splits drops its block-axis alignment.

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
    row: usize,
    grown: usize,
    end: Option<f32>,
    split_cells: &std::collections::HashSet<usize>,
) -> Vec<CellFragment> {
    // Only a forced break in the cell's own block flow splits the row there.
    if document.context.pagination_on_transposed_axes
        || document.nodes[row].display() != DisplayValue::TableRow
        || end.is_none()
        || !breaks_inside(document, cascade, grown)
    {
        return Vec::new();
    }
    let end = end.unwrap_or_default();
    document.nodes[row]
        .children
        .iter()
        .copied()
        .filter(|&cell| {
            let node = &document.nodes[cell];
            cell != grown
                && !split_cells.contains(&cell)
                && node.display() == DisplayValue::TableCell
                && sibling_renders(document, cell)
                && !node.used_writing_mode.is_vertical()
                && node.layout.location.y + node.layout.size.height <= end + 0.001
                && !breaks_inside(document, cascade, cell)
        })
        .filter_map(|cell| {
            let layout = &document.nodes[cell].layout;
            let top = layout.location.y;
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
/// children of `display: contents` boxes in place of them. Floats and
/// positioned boxes are not that content.
fn flow_children(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    parent: usize,
    out: &mut Vec<usize>,
) {
    for &child in &document.nodes[parent].children {
        let node = &document.nodes[child];
        if !sibling_renders(document, child)
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
        page_break_is_forced(document, child, computed.break_before) && before
            || page_break_is_forced(document, child, computed.break_after) && after
            || stacks_children_in_block_flow(document, child)
                && breaks_between(
                    document,
                    cascade,
                    child,
                    before || generated_on_side(cascade, child, true),
                    after || generated_on_side(cascade, child, false),
                )
    })
}
