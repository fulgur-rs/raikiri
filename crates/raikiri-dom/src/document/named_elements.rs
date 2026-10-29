//! Change snapshots for native named-element queries.

use std::sync::Arc;

use super::Document;

/// An opaque snapshot of document identity and named-element changes.
///
/// Compare snapshots to invalidate cached ID/name queries. A cloned or newly
/// restored document has a distinct identity, even when its nodes are equal.
#[derive(Debug, Clone)]
pub struct NamedElementVersion {
    identity: Arc<()>,
    generation: u64,
}

impl PartialEq for NamedElementVersion {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity) && self.generation == other.generation
    }
}

impl Eq for NamedElementVersion {}

#[derive(Debug)]
pub(super) struct Tracker {
    version: NamedElementVersion,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            version: NamedElementVersion {
                identity: Arc::new(()),
                generation: 0,
            },
        }
    }
}

impl Clone for Tracker {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Document {
    /// The current identity and revision of native named-element lookup inputs.
    ///
    /// Topology, element namespace and null-namespace `id`/`name` attribute
    /// mutations invalidate this snapshot, independently of topology tracking
    /// and layout flushing. Reading it does not allocate or walk the tree.
    pub fn named_element_version(&self) -> &NamedElementVersion {
        &self.named_elements.version
    }

    pub(super) fn note_named_element_change(&mut self) {
        if let Some(next) = self.named_elements.version.generation.checked_add(1) {
            self.named_elements.version.generation = next;
        } else {
            // Never let a wrapped revision compare equal to an older snapshot.
            self.named_elements = Tracker::default();
        }
    }
}

#[cfg(test)]
mod tests;
