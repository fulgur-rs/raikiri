use crate::Document;
use raikiri_traits::{LayoutError, NodeKind};

/// Maximum number of nested elements in the body subtree accepted by layout.
///
/// The body is the layout root and has depth 1. Elements above it, such as
/// `<html>`, are not part of the Taffy layout tree. Hidden descendants count
/// toward the limit because layout engines may still traverse their subtree.
///
/// The stack-safety guarantee for accepted depths applies to native targets.
/// WebAssembly hosts control the engine call-stack limit, so accepted-depth
/// safety there depends on the embedding runtime's stack configuration.
#[doc(hidden)]
pub const MAX_LAYOUT_DEPTH: usize = 256;

/// Most child layouts that may be nested while a layout pass runs.
///
/// The layouts of a chain of boxes nest one per element, plus a few for the
/// boxes above `<body>` and the wrappers a display generates; chains of every
/// display, positioning, writing mode, containment and fragmentation reach the
/// element depth plus four at most. Eight times the deepest tree layout accepts
/// ([`MAX_LAYOUT_DEPTH`]) leaves room for a box that nests its own layout a few
/// times, while a layout that never ends is stopped within a few megabytes of
/// stack instead of growing it without bound.
pub(crate) const MAX_LAYOUT_RECURSION: usize = MAX_LAYOUT_DEPTH * 8;

/// Reject a body subtree whose element depth exceeds [`MAX_LAYOUT_DEPTH`].
///
/// This check uses an explicit work stack so it remains safe for attacker-
/// controlled nesting. It follows document-tree children regardless of
/// computed display; detached template contents are outside the layout tree.
#[doc(hidden)]
pub fn validate_layout_depth(document: &Document) -> Result<(), LayoutError> {
    let Some(body_id) = super::find_body(document) else {
        return Ok(());
    };

    let mut pending = vec![(body_id, 1usize)];
    while let Some((node_id, element_depth)) = pending.pop() {
        let Some(node) = document.nodes.get(node_id) else {
            continue;
        };
        for &child_id in node.children.iter().rev() {
            let child_depth = if document
                .nodes
                .get(child_id)
                .is_some_and(|child| child.kind() == NodeKind::Element)
            {
                element_depth.saturating_add(1)
            } else {
                element_depth
            };
            if child_depth > MAX_LAYOUT_DEPTH {
                return Err(LayoutError::TreeDepthLimitExceeded {
                    limit: MAX_LAYOUT_DEPTH,
                    actual: child_depth,
                });
            }
            pending.push((child_id, child_depth));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
