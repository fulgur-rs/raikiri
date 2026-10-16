//! The page-area initial containing block, outside the source DOM.

use crate::Document;
use crate::node::{Node, NodeFlags};
use taffy::{Dimension, Size, Style};

/// The document element whose principal box is a child of the ICB.
/// Source parents and children remain the DOM tree throughout layout.
pub(crate) fn root_element(document: &Document) -> Option<usize> {
    document.nodes[document.root]
        .children
        .iter()
        .copied()
        .find(|&id| document.nodes[id].kind() == raikiri_traits::NodeKind::Element)
}

/// Capture the document-element role before box lowering and IFC selection.
pub(crate) fn prepare_root_element(document: &mut Document) {
    let root = root_element(document);
    for (id, node) in document.nodes.iter_mut().enumerate() {
        node.flags.set(NodeFlags::IS_ROOT_ELEMENT, Some(id) == root);
    }
}

/// Install a definite page-area box in the layout arena and return its key.
pub(crate) fn prepare(document: &mut Document, width: f32, height: f32) -> usize {
    let mut node = Node::new_element(
        "".into(),
        Style {
            display: taffy::Display::FlowRoot,
            size: Size {
                width: Dimension::length(width),
                height: Dimension::length(height),
            },
            ..Style::default()
        },
        None,
    );
    if let Some(root) = root_element(document) {
        node.children.push(root);
        node.used_writing_mode = document.nodes[root].used_writing_mode;
        node.style.direction = document.nodes[root].style.direction;
    }
    document.nodes.set_initial_containing_block(node)
}

#[cfg(test)]
mod tests;
