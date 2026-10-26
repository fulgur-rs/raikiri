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
use std::collections::HashMap;

/// The origins one walk of [`Document::layout_origin_of`] has resolved.
///
/// An out-of-flow box takes its origin from three boxes: the one it is
/// positioned against, the one it would sit in without being positioned, and
/// its coordinate parent, which is usually the first of them. In a chain of
/// positioned boxes all three are the box one level up, so each level asks the
/// level above it for the same origin three times, and a chain of `n` boxes
/// costs `3^n` steps. The origins are kept once resolved, so the chain costs
/// `n`.
///
/// A walk that meets no out-of-flow box is one chain of ancestors, which has
/// nothing to remember, so the map stays empty (and unallocated) until the
/// first out-of-flow box.
#[derive(Default)]
struct OriginWalk {
    resolved: HashMap<usize, Option<Point<f32>>>,
}

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
        self.layout_offset_of_at_depth(node, 0, &mut OriginWalk::default())
    }

    /// The node's border-box origin in the unfragmented layout coordinate
    /// space, following containing-block owners for out-of-flow boxes.
    #[doc(hidden)]
    pub fn layout_origin_of(&self, node: usize) -> Option<Point<f32>> {
        self.layout_origin_of_at_depth(node, 0, &mut OriginWalk::default())
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

    /// The deepest a walk goes before it gives up. A walk steps from a box to
    /// its offset and from the offset to the origin of another box, so it takes
    /// two steps for each box on the way, and meets each box once unless the
    /// coordinates form a cycle.
    fn origin_walk_limit(&self) -> usize {
        self.nodes.len().saturating_mul(2).saturating_add(2)
    }

    fn layout_offset_of_at_depth(
        &self,
        node: usize,
        depth: usize,
        walk: &mut OriginWalk,
    ) -> Option<Point<f32>> {
        if depth > self.origin_walk_limit() {
            return None;
        }
        let mut offset = self.nodes.layout_node(node)?.layout.location;
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
            .layout_origin_of_at_depth(coordinate.owner, depth + 1, walk)
            .unwrap_or(initial_owner);
        let static_owner = self
            .layout_origin_of_at_depth(coordinate.static_owner, depth + 1, walk)
            .unwrap_or(initial_static);
        if coordinate.follow_static_x {
            offset.x += (static_owner.x - initial_static.x) - (owner.x - initial_owner.x);
        }
        if coordinate.follow_static_y {
            offset.y += (static_owner.y - initial_static.y) - (owner.y - initial_owner.y);
        }
        Some(offset)
    }

    fn layout_origin_of_at_depth(
        &self,
        node: usize,
        depth: usize,
        walk: &mut OriginWalk,
    ) -> Option<Point<f32>> {
        if depth > self.origin_walk_limit() {
            return None;
        }
        // Only an out-of-flow box reaches the same box along several paths.
        // From the first one on, everything the walk resolves is kept, since a
        // box below the others may be met again from any of them.
        let positioned = self.oof_coordinates.contains_key(&node);
        if positioned || !walk.resolved.is_empty() {
            if let Some(&resolved) = walk.resolved.get(&node) {
                return resolved;
            }
            if positioned {
                // A box that asks for its own origin, through the boxes it is
                // positioned against, finds no origin there instead of
                // looping.
                walk.resolved.insert(node, None);
            }
        }
        let origin = self.resolve_layout_origin(node, depth, walk);
        if positioned || !walk.resolved.is_empty() {
            walk.resolved.insert(node, origin);
        }
        origin
    }

    fn resolve_layout_origin(
        &self,
        node: usize,
        depth: usize,
        walk: &mut OriginWalk,
    ) -> Option<Point<f32>> {
        #[cfg(test)]
        crate::layout::test_support::LAYOUT_ORIGIN_STEPS.with(|steps| steps.set(steps.get() + 1));
        let offset = self.layout_offset_of_at_depth(node, depth + 1, walk)?;
        let Some(parent) = self.layout_coordinate_parent_of(node) else {
            return Some(offset);
        };
        let parent_origin = self
            .layout_origin_of_at_depth(parent, depth + 1, walk)
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
