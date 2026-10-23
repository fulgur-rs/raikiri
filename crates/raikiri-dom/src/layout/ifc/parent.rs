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
use raikiri_layout_geometry::Point;
use raikiri_traits::NodeKind;

impl Document {
    /// The node `node`'s layout location is relative to: its DOM parent,
    /// except that the inline elements of an inline engine paragraph are
    /// skipped for a node that is not part of the paragraph's own content (one
    /// of its boxes), whose location is relative to the paragraph's root.
    #[doc(hidden)]
    pub fn layout_parent_of(&self, node: usize) -> Option<usize> {
        let parent = self.parent_of(node)?;
        if self.nodes[node].flags.contains(NodeFlags::IN_IFC_SUBTREE) {
            return Some(parent);
        }
        self.inclusive_ancestors(parent)
            .find(|&id| self.contributes_layout_offset(id))
    }

    /// The coordinate parent used to resolve a node's layout location.
    #[doc(hidden)]
    pub fn layout_coordinate_parent_of(&self, node: usize) -> Option<usize> {
        self.oof_coordinates
            .get(&node)
            .map(|coordinate| coordinate.owner)
            .or_else(|| self.layout_parent_of(node))
    }

    /// The node's local layout offset, including pagination movement of an
    /// automatic static position relative to its containing block.
    #[doc(hidden)]
    pub fn layout_offset_of(&self, node: usize) -> Option<Point<f32>> {
        self.layout_offset_of_at_depth(node, 0)
    }

    /// The node's border-box origin in the unfragmented layout coordinate
    /// space, following containing-block owners for out-of-flow boxes.
    #[doc(hidden)]
    pub fn layout_origin_of(&self, node: usize) -> Option<Point<f32>> {
        self.layout_origin_of_at_depth(node, 0)
    }

    pub(crate) fn begin_oof_layout(&mut self) {
        if self.layout_dirty {
            self.oof_coordinates.clear();
        }
        for coordinate in self.oof_coordinates.values_mut() {
            coordinate.initial_owner_origin = None;
            coordinate.initial_static_origin = None;
        }
    }

    pub(crate) fn finalize_oof_layout(&mut self) {
        let pending: Vec<_> = self
            .oof_coordinates
            .iter()
            .map(|(&node, coordinate)| (node, coordinate.owner, coordinate.static_owner))
            .collect();
        for (node, owner, static_owner) in pending {
            let owner_origin = self.layout_origin_of(owner).unwrap_or(Point::ZERO);
            let static_origin = self.layout_origin_of(static_owner).unwrap_or(Point::ZERO);
            if let Some(coordinate) = self.oof_coordinates.get_mut(&node) {
                coordinate.initial_owner_origin = Some(owner_origin);
                coordinate.initial_static_origin = Some(static_origin);
            }
        }
    }

    fn layout_offset_of_at_depth(&self, node: usize, depth: usize) -> Option<Point<f32>> {
        if depth > self.nodes.len().saturating_add(2) {
            return None;
        }
        let mut offset = self.nodes.layout_node(node)?.unrounded_layout.location;
        let Some(coordinate) = self.oof_coordinates.get(&node) else {
            return Some(offset);
        };
        let (Some(initial_owner), Some(initial_static)) = (
            coordinate.initial_owner_origin,
            coordinate.initial_static_origin,
        ) else {
            return Some(offset);
        };
        let owner = self
            .layout_origin_of_at_depth(coordinate.owner, depth + 1)
            .unwrap_or(initial_owner);
        let static_owner = self
            .layout_origin_of_at_depth(coordinate.static_owner, depth + 1)
            .unwrap_or(initial_static);
        if coordinate.follow_static_x {
            offset.x += (static_owner.x - initial_static.x) - (owner.x - initial_owner.x);
        }
        if coordinate.follow_static_y {
            offset.y += (static_owner.y - initial_static.y) - (owner.y - initial_owner.y);
        }
        Some(offset)
    }

    fn layout_origin_of_at_depth(&self, node: usize, depth: usize) -> Option<Point<f32>> {
        if depth > self.nodes.len().saturating_add(2) {
            return None;
        }
        let offset = self.layout_offset_of_at_depth(node, depth + 1)?;
        let Some(parent) = self.layout_coordinate_parent_of(node) else {
            return Some(offset);
        };
        let parent_origin = self
            .layout_origin_of_at_depth(parent, depth + 1)
            .unwrap_or(Point::ZERO);
        Some(Point {
            x: parent_origin.x + offset.x,
            y: parent_origin.y + offset.y,
        })
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
        self.ancestors(node)
            .find(|&id| self.nodes[id].flags.contains(NodeFlags::IS_IFC_ROOT))
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
