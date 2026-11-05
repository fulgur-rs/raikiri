//! Leading replaced or size-contained content of a column with no gap.

use super::*;
use crate::layout::context::LayoutSession;
use crate::layout::page::pagination_child_order_borrowed;

const TOLERANCE: f32 = 0.001;

/// The first replaced or size-contained piece at the column's block-start edge. Ordinary
/// lines and cuttable boxes remain on their existing pagination paths.
pub(super) fn first_piece(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    column: usize,
) -> Option<f32> {
    let node = &document.nodes[column];
    let computed = &cascade.computed[column];
    if node.display() != DisplayValue::Flex
        || !matches!(
            computed.flex_direction,
            FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
        )
        || computed.flex_wrap != FlexWrapValue::NoWrap
        || computed.overflow.x != OverflowValue::Visible
        || computed.overflow.y != OverflowValue::Visible
        || node.used_writing_mode.is_vertical()
        || node.layout.border.top + node.layout.padding.top > TOLERANCE
    {
        return None;
    }
    let first = first_box(document, cascade, column)?;
    let piece = unbreakable_piece(document, cascade, body_id, column, first, 0)?;
    // Only scan later descendants once a leading replaced piece was found.
    container_can_be_judged(document, cascade, column).then_some(piece)
}

fn unbreakable_piece(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    parent: usize,
    id: usize,
    depth: usize,
) -> Option<f32> {
    let node = &document.nodes[id];
    let top = pagination_child_parent_y(
        document,
        parent,
        id,
        normal_flow_y(document, cascade, body_id, id),
    );
    if depth > FIRST_PIECE_DEPTH
        || node.kind() != NodeKind::Element
        || node.used_writing_mode.is_vertical()
        || node.multicol.is_some()
        || node.has_before_or_after_content
        || !matches!(
            node.position(),
            PositionValue::Static | PositionValue::Relative
        )
        || top.abs() > TOLERANCE
    {
        return None;
    }
    if is_replaced_for_pagination(document, id)
        || size_contained_for_pagination(BoxTree::for_layout(document), id)
    {
        return Some(node.layout.size.height);
    }
    // Only a leading chain of ordinary content-sized blocks is transparent.
    // A line, a cuttable first box or a clip is a separate place to break.
    if !is_plain_block_box(document, id)
        || node.is_ifc_root()
        || !height_follows_content(document, id)
        || cascade.computed[id].overflow.x != OverflowValue::Visible
        || cascade.computed[id].overflow.y != OverflowValue::Visible
        || node.layout.border.top + node.layout.padding.top > TOLERANCE
    {
        return None;
    }
    let child = first_box(document, cascade, id)?;
    unbreakable_piece(document, cascade, body_id, id, child, depth + 1)
}

fn first_box(document: &Document, cascade: &CascadeResult, parent: usize) -> Option<usize> {
    for &child in pagination_child_order_borrowed(document, cascade, parent).iter() {
        if is_floating_box_for_pagination(document, cascade, child) {
            return None;
        }
        let mut boxes = Vec::new();
        push_in_flow_boxes_rev(document, cascade, child, &mut boxes);
        if let Some(first) = boxes.pop() {
            return Some(first);
        }
    }
    None
}
