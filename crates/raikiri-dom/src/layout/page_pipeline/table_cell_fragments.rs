//! A single-line cell can keep its text while its spare height continues.

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
    /// Retain the cell's height, moving only its alignment room below the text.
    pub(super) fn apply(
        self,
        document: &mut super::super::context::LayoutSession<'_>,
    ) -> (usize, f32) {
        let mut layout = document.nodes[self.cell].layout;
        layout.padding.top -= self.shift;
        layout.padding.bottom += self.shift;
        document.store_layout(self.cell, &layout);
        (self.cell, self.shift)
    }
}

/// Qualify only a real, one-cell, one-row table with one direct text line.
/// Other table contents keep their existing row-boundary decisions.
pub(super) fn fitting(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    row: usize,
    top: f32,
    page_start: f32,
    page_end: f32,
) -> Option<CellFragment> {
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
        if only_box(document, owner)? != row {
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
    let shift = match node.table_vertical_align {
        VerticalAlign::Top | VerticalAlign::Middle | VerticalAlign::Bottom => {
            layout.padding.top - padding.top
        }
        _ => return None,
    };
    (shift.is_finite() && shift >= 0.0).then_some(CellFragment { cell, shift })
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
