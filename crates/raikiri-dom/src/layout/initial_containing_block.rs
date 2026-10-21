//! The page-area initial containing block, outside the source DOM.

use crate::Document;
use crate::LayoutStyle as Style;
use crate::node::{Node, NodeFlags};
use raikiri_style::CascadeResult;
use raikiri_style::property::{Contain, DisplayValue};
use taffy::{Dimension, Size};

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

/// The element whose writing mode and direction the ICB takes: the first
/// HTML `body` child of an `html` root whose `display` is not `none`, else
/// the root (CSS Writing Modes 3 §8, principal writing mode). Containment
/// on the root or that body disables the propagation (CSS Containment
/// 2 §2).
fn principal_element(document: &Document, cascade: &CascadeResult, root: usize) -> usize {
    const HTML: Option<&str> = Some("http://www.w3.org/1999/xhtml");
    let is_html = |id: usize, name: &str| {
        document.nodes[id].tag_name() == Some(name) && document.element_namespace_uri(id) == HTML
    };
    let contained = |id: usize| cascade.computed[id].contain != Contain::NONE;
    if !is_html(root, "html") || contained(root) {
        return root;
    }
    document.nodes[root]
        .children
        .iter()
        .copied()
        .find(|&child| {
            is_html(child, "body") && cascade.computed[child].display != DisplayValue::None
        })
        .filter(|&body| !contained(body))
        .unwrap_or(root)
}

/// Install a definite page-area box in the layout arena and return its key.
pub(crate) fn prepare(
    document: &mut Document,
    cascade: &CascadeResult,
    width: f32,
    height: f32,
) -> usize {
    let mut style = Style {
        // The native root establishes its BFC and resolves a Block root's
        // definite dimensions before its children need percentage bases.
        display: taffy::Display::Block,
        size: Size {
            width: Dimension::length(width),
            height: Dimension::length(height),
        },
        ..Style::default()
    };
    let mut node = Node::new_element("".into(), style.clone(), None);
    let mut recipe_direction = taffy::Direction::Ltr;
    if let Some(root) = root_element(document) {
        node.children.push(root);
        let principal = principal_element(document, cascade, root);
        node.used_writing_mode = document.nodes[principal].used_writing_mode;
        style.direction = document
            .layout_style_for_key(principal)
            .expect("principal element has a style")
            .direction;
        recipe_direction = super::bridge::direction_for_axes(
            &cascade.computed[principal],
            super::bridge::uses_vertical_axes(cascade, principal),
        );
    }
    let used_writing_mode = node.used_writing_mode;
    let key = document.set_initial_containing_block(node, style);
    let recipe = super::box_style_data::recipes::initial_containing_block(
        width,
        height,
        super::box_style_data::recipes::BoxFacts {
            direction: recipe_direction,
            metadata: raikiri_layout_style::StyleMetadata {
                flags: raikiri_layout_style::StyleFlags::BLOCK,
                used_writing_mode,
                claims: None,
            },
        },
    );
    document
        .box_style_data
        .publish_recipe(crate::node::arena::BoxKey::InitialContainingBlock, recipe);
    key
}

#[cfg(test)]
mod tests;
