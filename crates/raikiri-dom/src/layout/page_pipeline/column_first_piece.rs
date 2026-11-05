//! A trailing column item's first piece supplies a missing page boundary.

use super::*;
use crate::layout::context::LayoutSession;
use crate::layout::page::pagination_child_order_borrowed;
use raikiri_style::property::ContentAlignmentValue;

/// Whether the last item needs a new page for its first indivisible piece.
/// Later content may still fragment; the item does not become monolithic.
pub(super) fn moves(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    item: usize,
    fit: &BoxFit,
) -> bool {
    piece(document, cascade, body_id, item).is_some_and(|end| {
        end <= fit.next_page_step + 0.001
            && avoided_box_moves(
                fit.flow_y,
                fit.margin_top,
                fit.flow_y + end,
                fit.page_start,
                fit.page_end,
            )
    })
}

fn piece(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    item: usize,
) -> Option<f32> {
    let parent = BoxTree::for_layout(document).parent(item, BoxParentScope::Document)?;
    let node = &document.nodes[item];
    let column = &document.nodes[parent];
    let computed = &cascade.computed[parent];
    if node.kind() != NodeKind::Element
        || node.used_writing_mode.is_vertical()
        || !matches!(node.position(), PositionValue::Static)
        || !column.display().is_flex_container()
        || column.used_writing_mode.is_vertical()
        || computed.flex_direction != FlexDirectionValue::Column
        || computed.flex_wrap != FlexWrapValue::NoWrap
        || !matches!(
            computed.justify_content,
            ContentAlignmentValue::Normal
                | ContentAlignmentValue::Start
                | ContentAlignmentValue::FlexStart
        )
        || !height_follows_content(document, parent)
        || document.block_axis(parent).aspect_ratio().is_some()
        || !placed_in_block_flow(document, cascade, parent, body_id)
        || pagination_child_order_borrowed(document, cascade, parent)
            .iter()
            .any(|&child| {
                is_in_flow_flex_child_for_pagination(document, parent, child)
                    && cascade.computed[child].order != 0
            })
    {
        return None;
    }
    first_piece_end(document, cascade, body_id, item, 0)
}

/// Whether this paragraph can use line-break constraints inside its item.
/// The caller checks the cached trailing identity and runs this only before
/// the paragraph's first orphans/widows check.
pub(super) fn line_paragraph(
    document: &LayoutSession<'_>,
    cascade: &CascadeResult,
    body_id: usize,
    item: usize,
) -> bool {
    height_follows_content(document, item) && piece(document, cascade, body_id, item).is_some()
}

#[cfg(test)]
mod tests;
