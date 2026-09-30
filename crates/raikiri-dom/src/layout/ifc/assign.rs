//! Choose the blocks laid out by the shodo inline engine.

use super::projection::project_ifc;
use super::root::IfcRoot;
use crate::Document;
use crate::node::NodeFlags;
use raikiri_style::CascadeResult;
use raikiri_style::ComputedColumnWidth;
use raikiri_style::property::{ColumnCountValue, DisplayValue};
use raikiri_traits::NodeKind;

fn is_block_container(display: DisplayValue) -> bool {
    matches!(display, DisplayValue::Block | DisplayValue::FlowRoot)
}

fn is_horizontal(cascade: &CascadeResult, idx: usize) -> bool {
    cascade.computed[idx].cssom_writing_mode == raikiri_style::property::WritingMode::HorizontalTb
}

fn is_multicol(cascade: &CascadeResult, idx: usize) -> bool {
    let cv = &cascade.computed[idx];
    !matches!(cv.column_count, ColumnCountValue::Auto)
        || !matches!(cv.column_width, ComputedColumnWidth::Auto)
}

/// Every in-flow child of `parent` other than `idx` is block-level, so the
/// parent does not lay `idx` out inside an inline line box.
fn parent_holds_only_blocks(doc: &Document, cascade: &CascadeResult, parent: usize) -> bool {
    doc.nodes[parent].children.iter().all(|&child| {
        let node = &doc.nodes[child];
        if !node.is_in_document() {
            return true;
        }
        match node.kind() {
            NodeKind::Text => node
                .text_content()
                .is_none_or(|text| text.chars().all(|c| c.is_ascii_whitespace())),
            NodeKind::Element => {
                // Only an inline-level sibling puts the paragraph inside an
                // inline formatting context of its parent; a block-level box
                // of any inner display type does not.
                !matches!(
                    cascade.computed[child].display,
                    DisplayValue::Inline
                        | DisplayValue::InlineBlock
                        | DisplayValue::InlineFlex
                        | DisplayValue::InlineGrid
                        | DisplayValue::InlineTable
                        | DisplayValue::Contents
                )
            }
            _ => true,
        }
    })
}

fn has_visible_text(doc: &Document, idx: usize) -> bool {
    let mut stack = doc.nodes[idx].children.clone();
    while let Some(id) = stack.pop() {
        let node = &doc.nodes[id];
        if !node.is_in_document() {
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                if node
                    .text_content()
                    .is_some_and(|text| text.chars().any(|c| !c.is_ascii_whitespace()))
                {
                    return true;
                }
            }
            NodeKind::Element => stack.extend(node.children.iter().copied()),
            _ => {}
        }
    }
    false
}

/// Clear every IFC mark, then mark the eligible roots and their subtrees.
pub(crate) fn assign_ifc_roots(doc: &mut Document, cascade: &CascadeResult) {
    for node in &mut doc.nodes {
        node.flags
            .remove(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE);
        node.ifc = None;
    }
    // Take the engine state out so `project_ifc` can borrow the document.
    let Some(mut state) = doc.ifc.take() else {
        return;
    };
    // The roots are rebuilt on every pass, so a cached layout would skip the
    // measure callback that fills their lines.
    doc.layout_dirty = true;
    let mut taken = vec![false; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        let node = &doc.nodes[idx];
        if node.kind() != NodeKind::Element
            || !node.is_in_document()
            || cascade.computed[idx].display != DisplayValue::Block
            || !is_horizontal(cascade, idx)
            || taken[idx]
        {
            continue;
        }
        let Some(parent) = doc.parent_of(idx) else {
            continue;
        };
        if doc.nodes[parent].kind() != NodeKind::Element
            || !is_block_container(cascade.computed[parent].display)
            || !parent_holds_only_blocks(doc, cascade, parent)
        {
            continue;
        }
        // The root itself counts: a multicol container is laid out by its own
        // dispatch, which would find no children once they are hidden.
        let mut ancestor = Some(idx);
        let mut blocked = false;
        while let Some(id) = ancestor {
            if is_multicol(cascade, id) || taken[id] {
                blocked = true;
                break;
            }
            ancestor = doc.parent_of(id);
        }
        if blocked || !has_visible_text(doc, idx) {
            continue;
        }
        let Ok(projected) = project_ifc(
            doc,
            cascade,
            idx,
            &mut state.layout_cx,
            &state.fonts,
            &state.limits,
        ) else {
            continue;
        };
        doc.nodes[idx].flags.insert(NodeFlags::IS_IFC_ROOT);
        doc.nodes[idx].ifc = Some(Box::new(IfcRoot::new(projected)));
        let mut stack = doc.nodes[idx].children.clone();
        while let Some(id) = stack.pop() {
            doc.nodes[id].flags.insert(NodeFlags::IN_IFC_SUBTREE);
            taken[id] = true;
            stack.extend(doc.nodes[id].children.iter().copied());
        }
        taken[idx] = true;
    }
    doc.ifc = Some(state);
}

#[cfg(test)]
mod tests;
