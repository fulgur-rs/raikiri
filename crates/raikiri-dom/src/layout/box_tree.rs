//! Parent boxes in the cascade's box tree.

use super::ifc::assign::is_layout_root;
use crate::Document;
use raikiri_style::CascadeResult;
use raikiri_style::property::DisplayValue;
use raikiri_traits::NodeKind;

/// The boundary at which parent-box discovery stops.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoxParentScope {
    /// Allow a document or fragment ancestor as the containing boundary.
    Document,
    /// Stop at the body layout root, returning only element ancestors.
    LayoutRoot,
}

/// The nearest ancestor box, skipping `display: contents`.
///
/// [`BoxParentScope::LayoutRoot`] keeps the body even when its specified
/// display is `contents`, and stops without a box at a non-element boundary.
/// A missing cascade entry is treated as a boundary, as paint discovery
/// permits partial cascades.
pub(crate) fn box_parent(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    scope: BoxParentScope,
) -> Option<usize> {
    let parent = document.ancestors(node_id).find(|&id| {
        if scope == BoxParentScope::LayoutRoot && document.nodes[id].kind() != NodeKind::Element {
            return true;
        }
        cascade
            .computed
            .get(id)
            .is_none_or(|cv| cv.display != DisplayValue::Contents)
            || (scope == BoxParentScope::LayoutRoot && is_layout_root(document, id))
    })?;
    (scope == BoxParentScope::Document || document.nodes[parent].kind() == NodeKind::Element)
        .then_some(parent)
}

#[cfg(test)]
mod tests;
