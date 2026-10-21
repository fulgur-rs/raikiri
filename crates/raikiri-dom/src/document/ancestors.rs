//! Ancestor positions whose parent lookup can precede or follow a mutation.

use super::Document;

/// The parent graph followed by an ancestor walk.
#[derive(Clone, Copy)]
pub(crate) enum ParentRelation {
    Source,
    Layout,
}

impl ParentRelation {
    fn parent_of(self, document: &Document, node: usize) -> Option<usize> {
        match self {
            Self::Source => document.parent_of(node),
            Self::Layout => document.layout_parent_of(node),
        }
    }
}

/// An ancestor position that does not borrow the document between steps.
pub(crate) struct AncestorCursor {
    current: Option<usize>,
    relation: ParentRelation,
}

impl AncestorCursor {
    pub(crate) fn new(current: Option<usize>, relation: ParentRelation) -> Self {
        Self { current, relation }
    }

    pub(crate) fn current(&self) -> Option<usize> {
        self.current
    }

    /// Read the current node's parent now, at the caller's chosen mutation boundary.
    pub(crate) fn advance(&mut self, document: &Document) {
        self.current = self
            .current
            .and_then(|node| self.relation.parent_of(document, node));
    }
}

impl Document {
    /// Iterate over layout parents, nearest first, excluding `node` itself.
    pub(crate) fn layout_ancestors(&self, node: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(ParentRelation::Layout.parent_of(self, node), |&id| {
            ParentRelation::Layout.parent_of(self, id)
        })
    }

    /// Iterate over `node` and then its layout parents, nearest first.
    pub(crate) fn inclusive_layout_ancestors(
        &self,
        node: usize,
    ) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(node), |&id| ParentRelation::Layout.parent_of(self, id))
    }
}

#[cfg(test)]
mod tests;
