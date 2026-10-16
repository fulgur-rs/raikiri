//! Parent boxes in an explicit cascade query or a captured layout pass.

use super::ifc::assign::is_layout_root;
use crate::Document;
use crate::node::BoxPropertiesRef;
use raikiri_style::CascadeResult;
use raikiri_style::property::DisplayValue;
use raikiri_traits::NodeKind;

/// The boundary at which parent-box discovery stops.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoxParentScope {
    /// Allow a document or fragment ancestor as the containing boundary.
    Document,
    /// Return only element ancestors, stopping before the document boundary.
    LayoutRoot,
}

/// The nearest ancestor box, skipping `display: contents`.
///
/// [`BoxParentScope::LayoutRoot`] stops before a non-element boundary.
/// The document element is blockified, so a contents root still has a box.
/// A missing cascade entry is treated as a boundary, as paint discovery
/// permits partial cascades.
pub(crate) fn box_parent(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    scope: BoxParentScope,
) -> Option<usize> {
    BoxTree::for_cascade(document, cascade).parent(node_id, scope)
}

/// A box tree with one identity source for the entire operation.
///
/// Explicit CSS queries can run before layout or inspect a caller-selected
/// cascade. Layout operations use the identity captured by the bridge.
#[derive(Clone, Copy)]
pub(crate) enum BoxTree<'a> {
    /// The boxes captured for a layout pass, including anonymous boxes.
    Layout(&'a Document),
    /// A query of an explicit cascade, independent of retained layout metadata.
    Cascade(&'a Document, &'a CascadeResult),
}

impl<'a> BoxTree<'a> {
    pub(crate) fn for_layout(document: &'a Document) -> Self {
        Self::Layout(document)
    }

    pub(crate) fn for_cascade(document: &'a Document, cascade: &'a CascadeResult) -> Self {
        Self::Cascade(document, cascade)
    }

    pub(crate) fn properties(self, id: usize) -> Option<BoxPropertiesRef<'a>> {
        match self {
            Self::Layout(document) => document
                .nodes
                .layout_node(id)
                .map(|node| node.box_properties()),
            Self::Cascade(document, cascade) => cascade
                .computed
                .get(id)
                .map(BoxPropertiesRef::from_computed)
                .map(|identity| {
                    if is_layout_root(document, id) {
                        identity.blockify_root()
                    } else {
                        identity
                    }
                }),
        }
    }

    pub(crate) fn identity(self, id: usize) -> BoxPropertiesRef<'a> {
        match self {
            Self::Layout(document) => document.nodes[id].box_properties(),
            Self::Cascade(document, cascade) => {
                let identity = BoxPropertiesRef::from_computed(&cascade.computed[id]);
                if is_layout_root(document, id) {
                    identity.blockify_root()
                } else {
                    identity
                }
            }
        }
    }

    pub(crate) fn display(self, id: usize) -> Option<DisplayValue> {
        self.properties(id).map(BoxPropertiesRef::display)
    }

    fn document(self) -> &'a Document {
        match self {
            Self::Layout(document) | Self::Cascade(document, _) => document,
        }
    }

    pub(crate) fn parent(self, node_id: usize, scope: BoxParentScope) -> Option<usize> {
        let document = self.document();
        let parent = document.ancestors(node_id).find(|&id| {
            if scope == BoxParentScope::LayoutRoot && document.nodes[id].kind() != NodeKind::Element
            {
                return true;
            }
            self.display(id)
                .is_none_or(|display| display != DisplayValue::Contents)
                || (scope == BoxParentScope::LayoutRoot && is_layout_root(document, id))
        })?;
        (scope == BoxParentScope::Document || document.nodes[parent].kind() == NodeKind::Element)
            .then_some(parent)
    }
}

#[cfg(test)]
mod tests;
