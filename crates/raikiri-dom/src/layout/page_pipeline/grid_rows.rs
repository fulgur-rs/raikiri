//! First-row constraints that keep the items of the first row of a grid, or
//! of the line of a single-line flex container, together across page
//! boundaries.

use super::*;
use crate::layout::context::LayoutSession;

const TOLERANCE: f32 = 0.001;

#[derive(Clone, Copy)]
pub(super) struct FirstRow {
    start: f32,
    end: f32,
}

impl FirstRow {
    pub(super) fn whole_moves(
        self,
        flow_y: f32,
        margin_top: f32,
        page_start: f32,
        page_end: f32,
    ) -> bool {
        self.start <= TOLERANCE
            && avoided_box_moves(flow_y, margin_top, flow_y + self.end, page_start, page_end)
    }

    /// A padded row breaks after its leading edges, rather than moving them.
    /// Zero means its area already begins exactly on the next page boundary.
    pub(super) fn content_shift(self, flow_y: f32, page_start: f32, page_end: f32) -> Option<f32> {
        let top = flow_y + self.start;
        (self.start > TOLERANCE
            && top > page_start + TOLERANCE
            && top <= page_end + TOLERANCE
            && flow_y + self.end > page_end + TOLERANCE)
            .then_some((page_end - top).max(0.0))
    }
}

/// Read a guarded first-row constraint without changing ordinary paragraph
/// first-piece rules. Missing or stale placement metadata leaves the existing
/// pagination path in charge. The collector has already checked static or
/// relative positioning through its plain-container eligibility.
pub(super) fn first_row(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    grid: usize,
) -> Option<FirstRow> {
    let node = &document.nodes[grid];
    let computed = &cascade.computed[grid];
    let flex_line = node.display() == DisplayValue::Flex
        && matches!(
            computed.flex_direction,
            FlexDirectionValue::Row | FlexDirectionValue::RowReverse
        )
        && computed.flex_wrap == FlexWrapValue::NoWrap;
    // A single column of several items stacks them as block flow, which
    // breaks between them; one item alone is the first row.
    let grid_rows = node.display() == DisplayValue::Grid
        && (node.grid_column_count > 1 || node.grid_item_rows.len() == 1);
    if !(flex_line || grid_rows)
        || node.used_writing_mode.is_vertical()
        || document.context.pagination_on_transposed_axes
        || !height_follows_content(document, grid)
        || !grid_rows_size_to_content(document, grid)
        || computed.overflow.x != OverflowValue::Visible
        || computed.overflow.y != OverflowValue::Visible
        || selected_page_name(cascade, grid).is_some()
        || has_mandatory_break_inside(document, cascade, grid)
        || !plain_block_ancestors(document, cascade, body_id, grid)
        || !readable_subtree(document, cascade, grid)
    {
        return None;
    }
    if node.display() == DisplayValue::Flex {
        return first_flex_line(document, cascade, body_id, grid);
    }
    let rows = &node.grid_item_rows;
    if rows.len() != node.grid_item_row_starts.len()
        || rows
            .iter()
            .zip(node.grid_item_row_starts.iter())
            .any(|(row, &(item, start))| {
                row.item != item || row.start != start || row.start.checked_add(1) != Some(row.end)
            })
    {
        return None;
    }
    let first = rows.iter().min_by_key(|row| row.start)?;
    let start = first.block_start;
    let edges = node.layout.border.top + node.layout.padding.top;
    if !start.is_finite()
        || (start - edges).abs() > TOLERANCE
        || rows.iter().any(|row| {
            !row.block_start.is_finite()
                || row.block_start < start - TOLERANCE
                || (row.start == first.start && (row.block_start - start).abs() > TOLERANCE)
        })
    {
        return None;
    }
    let end = rows
        .iter()
        .filter(|row| row.start == first.start)
        .filter_map(|row| {
            unbreakable_piece(document, cascade, row.item)
                .map(|end| normal_flow_y(document, cascade, body_id, row.item) + end)
        })
        .reduce(f32::max)?;
    (end.is_finite() && end > start).then_some(FirstRow { start, end })
}

/// The first line of a single-line row flex container whose items no break
/// goes through, all of them at the top of its content box. Like the first
/// row of a grid, the line breaks after the padding or border above it
/// rather than leaving them alone, and a container that already starts a
/// page sends the line on to the next one.
fn first_flex_line(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    flex: usize,
) -> Option<FirstRow> {
    let node = &document.nodes[flex];
    let start = node.layout.border.top + node.layout.padding.top;
    let mut end = None::<f32>;
    for item in flex_items(document, flex)? {
        let top = normal_flow_y(document, cascade, body_id, item);
        if (top - start).abs() > TOLERANCE {
            return None;
        }
        // A box that only avoids a break inside it splits on the page it
        // moves to, as browsers print it, so only one that no break goes
        // through takes the line past the edges.
        if !is_monolithic_for_pagination_with_transposed_axes(document, cascade, item, false) {
            return None;
        }
        let piece = top + document.nodes[item].layout.size.height;
        end = Some(end.map_or(piece, |end| end.max(piece)));
    }
    let end = end?;
    (start > TOLERANCE && end.is_finite() && end > start).then_some(FirstRow { start, end })
}

/// The in-flow items of the flex container `flex`, or `None` when text of its
/// own makes an anonymous item.
pub(super) fn flex_items(document: &Document, flex: usize) -> Option<Vec<usize>> {
    let mut items = Vec::new();
    for &child in &document.nodes[flex].children {
        let node = &document.nodes[child];
        match &node.data {
            // Only white space that the white-space property can affect
            // makes no anonymous item.
            crate::node::NodeData::Text(text)
                if text
                    .text_content
                    .chars()
                    .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}')) => {}
            crate::node::NodeData::Element(_) if node.display() == DisplayValue::None => {}
            crate::node::NodeData::Element(_) if node.display() != DisplayValue::Contents => {
                items.push(child);
            }
            crate::node::NodeData::Comment(_)
            | crate::node::NodeData::ProcessingInstruction { .. } => {}
            _ => return None,
        }
    }
    Some(items)
}

/// Whether the first item of the grid `grid` is a box that no break goes
/// through or that avoids a break inside it, which takes the grid's top
/// edges along where [`first_row`] cannot read the grid.
pub(super) fn first_item_is_unbreakable(
    document: &Document,
    cascade: &CascadeResult,
    grid: usize,
) -> bool {
    document.nodes[grid]
        .grid_item_rows
        .iter()
        .min_by_key(|row| row.start)
        .is_some_and(|row| {
            avoids_page_break_inside(cascade.computed[row.item].break_inside)
                || is_monolithic_for_pagination_with_transposed_axes(
                    document, cascade, row.item, false,
                )
        })
}

fn unbreakable_piece(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    item: usize,
) -> Option<f32> {
    let node = &document.nodes[item];
    if avoids_page_break_inside(cascade.computed[item].break_inside)
        || is_monolithic_for_pagination_with_transposed_axes(document, cascade, item, false)
    {
        return Some(node.layout.size.height);
    }
    let ifc = document.ifc_layout_node(item)?.ifc.as_ref()?;
    let lines = ifc.lines.as_ref()?;
    if !ifc.generated_blocks.is_empty()
        || !lines.placed_blocks.is_empty()
        || !lines.block_line_starts.is_empty()
    {
        return None;
    }
    unplaced_line(document, cascade, item)
        .filter(|line| line.atomic)
        .map(|line| line.end.min(node.layout.size.height))
}

fn plain_block_ancestors(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    grid: usize,
) -> bool {
    let mut parent = document.parent_of(grid);
    while let Some(id) = parent {
        let node = &document.nodes[id];
        if node.display() != DisplayValue::Contents
            && (!is_plain_block_box(document, id)
                || node.is_ifc_root()
                || node.multicol.is_some()
                || !height_follows_content(document, id)
                || is_floating_box_for_pagination(document, cascade, id)
                || cascade.computed[id].overflow.x != OverflowValue::Visible
                || cascade.computed[id].overflow.y != OverflowValue::Visible)
        {
            return false;
        }
        if id == body_id {
            return true;
        }
        parent = document.parent_of(id);
    }
    false // cov:ignore: Attached candidate chains reach the body or fail a block check above.
}

fn readable_subtree(document: &Document, cascade: &CascadeResult, grid: usize) -> bool {
    let mut pending = vec![grid];
    while let Some(id) = pending.pop() {
        let node = &document.nodes[id];
        if node.display() == DisplayValue::None {
            continue;
        }
        if matches!(
            node.position(),
            PositionValue::Absolute | PositionValue::Fixed
        ) || node.multicol.is_some()
            || node.has_before_or_after_content
            || node.size_containment.height
            || node.used_writing_mode.is_vertical()
            || is_floating_box_for_pagination(document, cascade, id)
        {
            return false;
        }
        pending.extend(node.children.iter().copied());
    }
    true
}
