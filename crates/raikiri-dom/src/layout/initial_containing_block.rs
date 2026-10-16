//! The page-area initial containing block, outside the source DOM.

use crate::Document;
use crate::node::{Node, NodeFlags};
use taffy::{Dimension, Size, Style};

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

/// The element whose writing mode and direction the ICB takes: the HTML
/// `body` element when the root is `html` and has one, else the root
/// (CSS Writing Modes 3 §8, principal writing mode).
fn principal_element(document: &Document, root: usize) -> usize {
    if document.nodes[root].tag_name() != Some("html") {
        return root;
    }
    document.nodes[root]
        .children
        .iter()
        .copied()
        .find(|&child| document.nodes[child].tag_name() == Some("body"))
        .unwrap_or(root)
}

/// Install a definite page-area box in the layout arena and return its key.
pub(crate) fn prepare(document: &mut Document, width: f32, height: f32) -> usize {
    let mut node = Node::new_element(
        "".into(),
        Style {
            // The native root establishes its BFC and resolves a Block root's
            // definite dimensions before its children need percentage bases.
            display: taffy::Display::Block,
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
        let principal = principal_element(document, root);
        node.used_writing_mode = document.nodes[principal].used_writing_mode;
        node.style.direction = document.nodes[principal].style.direction;
    }
    document.nodes.set_initial_containing_block(node)
}

#[cfg(test)]
mod tests;
