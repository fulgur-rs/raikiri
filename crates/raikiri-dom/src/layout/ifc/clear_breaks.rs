//! Immutable physical clear values for forced breaks within one IFC root.

use std::collections::HashMap;
use std::sync::Arc;

/// A node-ID index shared by the root's trial-layout copies.
/// Empty roots allocate no index, and repeated IDs retain their first value.
#[derive(Clone, Debug, Default)]
pub(crate) struct ClearBreakIndex(Option<Arc<HashMap<usize, taffy::Clear>>>);

impl FromIterator<(usize, taffy::Clear)> for ClearBreakIndex {
    fn from_iter<T: IntoIterator<Item = (usize, taffy::Clear)>>(iter: T) -> Self {
        let entries = iter.into_iter();
        let mut by_node = HashMap::with_capacity(entries.size_hint().0);
        for (node, clear) in entries {
            by_node.entry(node).or_insert(clear);
        }
        Self((!by_node.is_empty()).then(|| Arc::new(by_node)))
    }
}

impl ClearBreakIndex {
    /// Return this node's physical clear side, independent of lookup order.
    pub(super) fn get(&self, node: usize) -> Option<taffy::Clear> {
        self.0.as_ref()?.get(&node).copied()
    }
}
