//! Source DOM nodes and separately stored layout-only boxes.

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
    /// A layout-only anonymous table that wraps table parts outside any
    /// table, by its index among those tables.
    AnonymousTable(usize),
    /// The page-area initial containing block, outside the source DOM.
    InitialContainingBlock,
}

// A reserved layout key stays valid when source nodes or anonymous cells grow.
const INITIAL_CONTAINING_BLOCK_KEY: usize = usize::MAX;

/// Source DOM nodes, anonymous table boxes, and the page-area ICB.
///
/// The arena dereferences to the source nodes only, so length, iteration and
/// mutation of the DOM never see a layout-only box. Indexing with a layout
/// key resolves any box kind without exposing generated boxes as DOM nodes.
#[derive(Debug, Clone, Default)]
pub(crate) struct NodeArena {
    source: Vec<Node>,
    anonymous_tables: Vec<Node>,
    anonymous_cells: Vec<Node>,
    /// The key of the first anonymous table, followed by the anonymous
    /// cells: the number of source nodes when the boxes were set. Table
    /// fixup sets the boxes again on every layout pass, after any change to
    /// the DOM.
    anonymous_base: usize,
    initial_containing_block: Option<Box<Node>>,
}

impl NodeArena {
    /// What `key` names, or `None` when it names nothing in this arena.
    pub(crate) fn box_key(&self, key: usize) -> Option<BoxKey> {
        if key == INITIAL_CONTAINING_BLOCK_KEY {
            return self
                .initial_containing_block
                .as_ref()
                .map(|_| BoxKey::InitialContainingBlock);
        }
        if key < self.source.len() {
            return Some(BoxKey::Source(key));
        }
        let index = key.checked_sub(self.anonymous_base)?;
        if index < self.anonymous_tables.len() {
            return Some(BoxKey::AnonymousTable(index));
        }
        let index = index - self.anonymous_tables.len();
        (index < self.anonymous_cells.len()).then_some(BoxKey::AnonymousCell(index))
    }

    /// Whether `key` names a layout-only anonymous table cell.
    pub(crate) fn is_anonymous_cell(&self, key: usize) -> bool {
        matches!(self.box_key(key), Some(BoxKey::AnonymousCell(_)))
    }

    /// Whether `key` names a layout-only anonymous table.
    pub(crate) fn is_anonymous_table(&self, key: usize) -> bool {
        matches!(self.box_key(key), Some(BoxKey::AnonymousTable(_)))
    }

    /// The layout key of the anonymous cell at `index`.
    pub(crate) fn anonymous_cell_key(&self, index: usize) -> usize {
        self.anonymous_base + self.anonymous_tables.len() + index
    }

    /// The layout key of the anonymous table at `index`.
    pub(crate) fn anonymous_table_key(&self, index: usize) -> usize {
        self.anonymous_base + index
    }

    /// The node or generated box that `key` names.
    pub(crate) fn layout_node(&self, key: usize) -> Option<&Node> {
        match self.box_key(key)? {
            BoxKey::Source(id) => Some(&self.source[id]),
            BoxKey::AnonymousCell(index) => Some(&self.anonymous_cells[index]),
            BoxKey::AnonymousTable(index) => Some(&self.anonymous_tables[index]),
            BoxKey::InitialContainingBlock => self.initial_containing_block.as_deref(),
        }
    }

    /// Replace the page-area root without adding a source DOM node.
    pub(crate) fn set_initial_containing_block(&mut self, node: Node) -> usize {
        self.initial_containing_block = Some(Box::new(node));
        INITIAL_CONTAINING_BLOCK_KEY
    }

    /// The page-area root, when a page layout has prepared one.
    pub(crate) fn initial_containing_block_key(&self) -> Option<usize> {
        self.initial_containing_block
            .as_ref()
            .map(|_| INITIAL_CONTAINING_BLOCK_KEY)
    }

    /// Replace the anonymous cells of the previous pass. Their keys follow
    /// the source nodes there are now.
    pub(crate) fn set_anonymous_cells(&mut self, cells: Vec<Node>) {
        self.anonymous_cells = cells;
        self.anonymous_base = self.source.len();
    }

    /// Replace the anonymous tables of the previous pass. Setting them moves
    /// the keys of the anonymous cells, so table fixup sets the tables
    /// before it creates any cell.
    pub(crate) fn set_anonymous_tables(&mut self, tables: Vec<Node>) {
        self.anonymous_tables = tables;
        self.anonymous_base = self.source.len();
    }

    /// Every source node and generated box a layout pass may lay out.
    pub(crate) fn layout_nodes_mut(&mut self) -> impl Iterator<Item = &mut Node> {
        self.source
            .iter_mut()
            .chain(&mut self.anonymous_tables)
            .chain(&mut self.anonymous_cells)
            .chain(self.initial_containing_block.as_deref_mut())
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
            anonymous_tables: Vec::new(),
            anonymous_cells: Vec::new(),
            initial_containing_block: None,
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
            Some(BoxKey::AnonymousTable(index)) => &self.anonymous_tables[index],
            Some(BoxKey::InitialContainingBlock) => {
                self.initial_containing_block.as_deref().unwrap()
            }
            // An out-of-range key panics on the source nodes, as a slice would.
            Some(BoxKey::Source(_)) | None => &self.source[key],
        }
    }
}

impl IndexMut<usize> for NodeArena {
    fn index_mut(&mut self, key: usize) -> &mut Node {
        match self.box_key(key) {
            Some(BoxKey::AnonymousCell(index)) => &mut self.anonymous_cells[index],
            Some(BoxKey::AnonymousTable(index)) => &mut self.anonymous_tables[index],
            Some(BoxKey::InitialContainingBlock) => {
                self.initial_containing_block.as_deref_mut().unwrap()
            }
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
