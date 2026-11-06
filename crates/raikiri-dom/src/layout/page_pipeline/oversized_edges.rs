//! Single-item holders that move with an oversized paragraph edge.

use super::*;

#[cfg(test)]
mod tests;

/// The exterior table or inline flex box whose only paragraph is `paragraph`.
/// This qualification does not enable ordinary block fragmentation inside it.
pub(super) fn holder(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    paragraph: usize,
    body: usize,
) -> Option<usize> {
    if !eligible_box(document, cascade, paragraph) {
        return None;
    }
    let parent = document.parent_of(paragraph)?;
    if !only_child(document, parent, paragraph) {
        return None;
    }
    let holder = match document.nodes[parent].display() {
        DisplayValue::InlineFlex => parent,
        DisplayValue::TableCell => {
            if document.element_attribute(parent, "rowspan").is_some()
                || document.element_attribute(parent, "colspan").is_some()
            {
                return None;
            }
            let row = document.parent_of(parent)?;
            if document.nodes[row].display() != DisplayValue::TableRow
                || !only_child(document, row, parent)
                || !eligible_box(document, cascade, parent)
                || !eligible_box(document, cascade, row)
            {
                return None;
            }
            let mut child = row;
            let mut owner = document.parent_of(row)?;
            if matches!(
                document.nodes[owner].display(),
                DisplayValue::TableRowGroup
                    | DisplayValue::TableHeaderGroup
                    | DisplayValue::TableFooterGroup
            ) {
                if !only_child(document, owner, child) || !eligible_box(document, cascade, owner) {
                    return None;
                }
                child = owner;
                owner = document.parent_of(owner)?;
            }
            if document.nodes[owner].display() != DisplayValue::Table
                || !only_child(document, owner, child)
            {
                return None;
            }
            owner
        }
        _ => return None,
    };
    if !eligible_box(document, cascade, holder) {
        return None;
    }
    if document.nodes[holder].display() == DisplayValue::Table {
        return placed_in_block_flow(document, cascade, holder, body).then_some(holder);
    }
    let parent = document.layout_parent_of(holder)?;
    if !matches!(
        document.nodes[parent].display(),
        DisplayValue::Block | DisplayValue::FlowRoot | DisplayValue::ListItem
    ) || parent != body && !placed_in_block_flow(document, cascade, parent, body)
    {
        return None;
    }
    // Other inline content shares the atomic line and must keep its existing
    // placement. Block siblings are separate lines and may precede or follow it.
    document.nodes[parent]
        .children
        .iter()
        .all(|&child| {
            child == holder
                || !renders(document, child)
                || document.nodes[child].kind() == NodeKind::Element
                    && !document.nodes[child].display().is_inline_level()
                    && document.nodes[child].display() != DisplayValue::Contents
                    && !is_floating_box_for_pagination(document, cascade, child)
        })
        .then_some(holder)
}

fn eligible_box(
    document: &super::super::context::LayoutSession<'_>,
    cascade: &CascadeResult,
    node: usize,
) -> bool {
    height_follows_content(document, node)
        && matches!(document.nodes[node].position(), PositionValue::Static)
        && document.nodes[node].multicol.is_none()
        && !document.nodes[node].used_writing_mode.is_vertical()
        && !element_clips_overflow(document, cascade, node)
        && !is_floating_box_for_pagination(document, cascade, node)
}

fn only_child(document: &Document, parent: usize, child: usize) -> bool {
    document.nodes[parent]
        .children
        .iter()
        .all(|&sibling| sibling == child || !renders(document, sibling))
}

fn renders(document: &Document, node: usize) -> bool {
    if document.nodes[node].kind() == NodeKind::Text {
        return document.nodes[node]
            .text_content()
            .is_some_and(|text| !text.chars().all(|c| c.is_ascii_whitespace()));
    }
    sibling_renders(document, node)
}
