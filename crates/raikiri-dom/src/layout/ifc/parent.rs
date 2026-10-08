//! Where a node's layout location is measured from when its DOM parent is an
//! inline element of a paragraph laid out by the inline engine.
//!
//! A box of such a paragraph (a float, an atomic inline or a block) is laid
//! out relative to the paragraph's root, wherever it sits in the DOM, while
//! an inline element of the paragraph records a location of its own, relative
//! to its nearest inline ancestor with a box. A reader that adds up the
//! locations of DOM parents would count the inline elements above a box on
//! top of the box's own location.

use crate::Document;
use crate::node::NodeFlags;
use raikiri_traits::NodeKind;

impl Document {
    /// The node `node`'s layout location is relative to: its DOM parent,
    /// except that the inline elements of an inline engine paragraph are
    /// skipped for a node that is not part of the paragraph's own content (one
    /// of its boxes), whose location is relative to the paragraph's root.
    #[doc(hidden)]
    pub fn layout_parent_of(&self, node: usize) -> Option<usize> {
        let mut parent = self.parent_of(node)?;
        if self.nodes[node].flags.contains(NodeFlags::IN_IFC_SUBTREE) {
            return Some(parent);
        }
        while !self.contributes_layout_offset(parent) {
            parent = self.parent_of(parent)?;
        }
        Some(parent)
    }

    /// The nearest ancestor of `node` that is the root of an inline engine
    /// paragraph.
    #[doc(hidden)]
    pub fn ifc_root_of(&self, node: usize) -> Option<usize> {
        if let Some(key) = self
            .table_objects
            .paragraph_owner
            .get(node)
            .copied()
            .flatten()
        {
            return Some(key);
        }
        let mut current = self.parent_of(node);
        while let Some(id) = current {
            if self.nodes[id].flags.contains(NodeFlags::IS_IFC_ROOT) {
                return Some(id);
            }
            current = self.parent_of(id);
        }
        None
    }

    /// Whether the location of `node` is part of the position of its DOM
    /// children: `false` for an inline element of an inline engine paragraph,
    /// whose children are text and inline elements measured from the lines
    /// and boxes measured from the paragraph's root.
    #[doc(hidden)]
    pub fn contributes_layout_offset(&self, node: usize) -> bool {
        self.nodes.get(node).is_none_or(|node| {
            !(node.kind() == NodeKind::Element && node.flags.contains(NodeFlags::IN_IFC_SUBTREE))
        })
    }
}

#[cfg(test)]
mod tests;
