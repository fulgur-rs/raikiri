//! Opt-in topology notifications for consumers maintaining ownership graphs.

use rustc_hash::FxHashSet;

use super::Document;

#[derive(Debug, Default)]
pub(super) struct TreeChanges {
    pending: Option<FxHashSet<usize>>,
}

impl Clone for TreeChanges {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Document {
    /// Start tracking nodes whose parent or template-contents edge changes.
    ///
    /// Tracking is off by default and is not copied when the document is
    /// cloned. Calling this again clears pending notifications. Consumers
    /// also observe new arena slots through [`Self::node_count`].
    pub fn enable_tree_change_tracking(&mut self) {
        self.tree_changes.pending = Some(FxHashSet::default());
    }

    /// Drain changed node indices since tracking began or the previous drain.
    ///
    /// Each changed index occurs at most once, in unspecified order. Read its
    /// current edges after draining: a node may have moved more than once.
    /// The pending set is bounded by the arena size and is empty when tracking
    /// is disabled. Reading does not enable tracking.
    pub fn take_tree_changes(&mut self) -> Vec<usize> {
        self.tree_changes
            .pending
            .as_mut()
            .map(|pending| pending.drain().collect())
            .unwrap_or_default()
    }

    pub(super) fn note_tree_change(&mut self, node: usize) {
        if let Some(pending) = &mut self.tree_changes.pending {
            pending.insert(node);
        }
    }
}
