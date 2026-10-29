//! Scalar child-element counts for queried nonempty parents of one revision.
//!
//! The first count scans the child slice; later counts use average O(1) hash
//! access. Metadata is bounded by queried nonempty source parents of the
//! observed revision. Old metadata remains until the next count or runtime
//! drop; discarding it can cost O(previous entries). No wrappers are retained.

use std::collections::HashMap;

use raikiri_dom::document::NamedElementVersion;
use raikiri_dom::{Document, NodeKind};

#[derive(Default)]
pub(crate) struct ElementCounts {
    version: Option<NamedElementVersion>,
    counts: HashMap<usize, usize>,
}

impl ElementCounts {
    pub(super) fn count(&mut self, document: &Document, parent: usize) -> usize {
        if self.version.as_ref() != Some(document.named_element_version()) {
            // Release old buckets even when this query is invalid or empty.
            self.counts = HashMap::new();
            self.version = Some(document.named_element_version().clone());
        }
        let Some(node) = document.get_node(parent) else {
            return 0;
        };
        if node.children.is_empty() {
            return 0;
        }
        if let Some(&count) = self.counts.get(&parent) {
            return count;
        }
        let count = node
            .children
            .iter()
            .filter(|&&child| {
                document
                    .get_node(child)
                    .is_some_and(|n| n.kind() == NodeKind::Element)
            })
            .count();
        #[cfg(test)]
        super::tests::record_element_child_work(2, node.children.len());
        self.counts.insert(parent, count);
        count
    }
}

#[cfg(test)]
mod tests;
