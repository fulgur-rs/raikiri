//! The page-area initial containing block, outside the source DOM.

use crate::Document;
use crate::node::{Node, NodeFlags};
use raikiri_style::CascadeResult;

/// The document element whose principal box is a child of the ICB.
/// Source parents and children remain the DOM tree throughout layout.
pub(crate) fn root_element(document: &Document) -> Option<usize> {
    document.document_element_index()
}

/// Capture the document-element role before box lowering and IFC selection.
pub(crate) fn prepare_root_element(document: &mut Document) {
    let root = root_element(document);
    for (id, node) in document.nodes.iter_mut().enumerate() {
        node.flags.set(NodeFlags::IS_ROOT_ELEMENT, Some(id) == root);
    }
}

/// Re-evaluate principal facts from the current source inputs.
pub(super) fn recipe_facts(
    document: &Document,
    cascade: &CascadeResult,
) -> super::box_style_data::recipes::BoxFacts {
    let principal = root_element(document).map(|_| cascade.principal_element_index());
    super::box_style_data::recipes::BoxFacts {
        direction: principal.map_or(taffy::Direction::Ltr, |id| {
            super::bridge::direction_for_axes(
                &cascade.computed[id],
                super::bridge::uses_vertical_axes(cascade, id),
            )
        }),
        metadata: raikiri_layout_style::StyleMetadata {
            flags: raikiri_layout_style::StyleFlags::BLOCK,
            used_writing_mode: principal
                .map_or(raikiri_style::property::WritingMode::HorizontalTb, |id| {
                    document.nodes[id].used_writing_mode
                }),
            claims: None,
        },
    }
}

/// Install a definite page-area box in the layout arena and return its key.
pub(crate) fn prepare(
    document: &mut Document,
    cascade: &CascadeResult,
    width: f32,
    height: f32,
) -> usize {
    let facts = recipe_facts(document, cascade);
    // The native root establishes its BFC and resolves a Block root's
    // definite dimensions before its children need percentage bases.
    let mut node = Node::new_element("".into(), None);
    if let Some(root) = root_element(document) {
        node.children.push(root);
        node.used_writing_mode =
            document.nodes[cascade.principal_element_index()].used_writing_mode;
    }
    let key = document.set_initial_containing_block(node);
    let recipe = super::box_style_data::recipes::initial_containing_block(width, height, facts);
    document
        .box_style_data
        .publish_recipe(crate::node::arena::BoxKey::InitialContainingBlock, recipe);
    key
}

#[cfg(test)]
mod tests;
