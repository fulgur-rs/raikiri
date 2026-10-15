//! The node arena: source DOM nodes followed by layout-only boxes.

use super::Node;
use std::ops::{Deref, DerefMut, Index, IndexMut};

/// What a layout key names.
///
/// Layout keys are plain `usize` values because taffy node ids and fragment
/// records carry them as numbers. Every key is classified here, so no other
/// code compares a key against the arena length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoxKey {
    /// A source DOM node, by its arena index.
    Source(usize),
    /// A layout-only anonymous table cell, by its index among those cells.
    AnonymousCell(usize),
}

/// Source DOM nodes, followed by the anonymous table cells that table fixup
/// generated for the current layout pass.
///
/// The arena dereferences to the source nodes only, so length, iteration and
/// mutation of the DOM never see a layout-only box. Indexing with a layout
/// key resolves either kind, which lets layout code address an anonymous cell
/// by its own key, like any other box.
#[derive(Debug, Clone, Default)]
pub(crate) struct NodeArena {
    source: Vec<Node>,
    anonymous_cells: Vec<Node>,
    /// The key of the first anonymous cell: the number of source nodes when
    /// the cells were set. Table fixup sets the cells again on every layout
    /// pass, after any change to the DOM.
    anonymous_base: usize,
}

impl NodeArena {
    /// What `key` names, or `None` when it names nothing in this arena.
    pub(crate) fn box_key(&self, key: usize) -> Option<BoxKey> {
        if key < self.source.len() {
            return Some(BoxKey::Source(key));
        }
        let index = key.checked_sub(self.anonymous_base)?;
        (index < self.anonymous_cells.len()).then_some(BoxKey::AnonymousCell(index))
    }

    /// Whether `key` names a layout-only anonymous table cell.
    pub(crate) fn is_anonymous_cell(&self, key: usize) -> bool {
        matches!(self.box_key(key), Some(BoxKey::AnonymousCell(_)))
    }

    /// The layout key of the anonymous cell at `index`.
    pub(crate) fn anonymous_cell_key(&self, index: usize) -> usize {
        self.anonymous_base + index
    }

    /// The node `key` names, a source node or an anonymous cell.
    pub(crate) fn layout_node(&self, key: usize) -> Option<&Node> {
        match self.box_key(key)? {
            BoxKey::Source(id) => Some(&self.source[id]),
            BoxKey::AnonymousCell(index) => Some(&self.anonymous_cells[index]),
        }
    }

    /// Replace the anonymous cells of the previous pass. Their keys follow
    /// the source nodes there are now.
    pub(crate) fn set_anonymous_cells(&mut self, cells: Vec<Node>) {
        self.anonymous_cells = cells;
        self.anonymous_base = self.source.len();
    }

    /// Every node a layout pass may lay out: the source nodes, then the
    /// anonymous cells.
    pub(crate) fn layout_nodes_mut(&mut self) -> impl Iterator<Item = &mut Node> {
        self.source.iter_mut().chain(&mut self.anonymous_cells)
    }

    /// The anonymous cells of the current pass, in key order.
    #[cfg(test)]
    pub(crate) fn anonymous_cells(&self) -> &[Node] {
        &self.anonymous_cells
    }
}

impl From<Vec<Node>> for NodeArena {
    fn from(source: Vec<Node>) -> Self {
        Self {
            anonymous_base: source.len(),
            source,
            anonymous_cells: Vec::new(),
        }
    }
}

impl FromIterator<Node> for NodeArena {
    fn from_iter<I: IntoIterator<Item = Node>>(nodes: I) -> Self {
        Vec::from_iter(nodes).into()
    }
}

impl Deref for NodeArena {
    type Target = Vec<Node>;

    fn deref(&self) -> &Vec<Node> {
        &self.source
    }
}

impl DerefMut for NodeArena {
    fn deref_mut(&mut self) -> &mut Vec<Node> {
        &mut self.source
    }
}

impl Index<usize> for NodeArena {
    type Output = Node;

    fn index(&self, key: usize) -> &Node {
        match self.box_key(key) {
            Some(BoxKey::AnonymousCell(index)) => &self.anonymous_cells[index],
            // An out-of-range key panics on the source nodes, as a slice would.
            Some(BoxKey::Source(_)) | None => &self.source[key],
        }
    }
}

impl IndexMut<usize> for NodeArena {
    fn index_mut(&mut self, key: usize) -> &mut Node {
        match self.box_key(key) {
            Some(BoxKey::AnonymousCell(index)) => &mut self.anonymous_cells[index],
            Some(BoxKey::Source(_)) | None => &mut self.source[key],
        }
    }
}

impl<'a> IntoIterator for &'a NodeArena {
    type Item = &'a Node;
    type IntoIter = std::slice::Iter<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.source.iter()
    }
}

impl<'a> IntoIterator for &'a mut NodeArena {
    type Item = &'a mut Node;
    type IntoIter = std::slice::IterMut<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.source.iter_mut()
    }
}

#[cfg(test)]
mod tests;
