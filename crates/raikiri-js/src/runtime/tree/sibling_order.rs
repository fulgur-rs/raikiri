//! Lazy sibling positions for one native document revision.
//!
//! A parent's first query scans once; its second query indexes its children.
//! Cached position lookup uses average O(1) hash access. Scalar metadata is
//! bounded by queried parents and indexed children of the observed revision.
//! A changed version discards that metadata on the next query; releasing the
//! old index can cost O(previous entries). Element getters still inspect gaps.

use std::collections::{HashMap, HashSet};

use raikiri_dom::Document;
use raikiri_dom::document::NamedElementVersion;

/// One marker for each queried parent and one position for each indexed child.
/// No wrappers are retained, and unrelated arena history is never scanned.
#[derive(Default)]
pub(crate) struct SiblingIndex {
    version: Option<NamedElementVersion>,
    positions: HashMap<usize, usize>,
    seen_parents: HashSet<usize>,
}

impl SiblingIndex {
    pub(super) fn position(&mut self, document: &Document, node: usize) -> Option<(usize, usize)> {
        if self.version.as_ref() != Some(document.named_element_version()) {
            // Release the old epoch's buckets, including after replacement by
            // a smaller document. Even a detached/invalid query discards them.
            self.positions = HashMap::new();
            self.seen_parents = HashSet::new();
            self.version = Some(document.named_element_version().clone());
        }
        document.get_node(node)?;
        let parent = document.parent_of(node)?;
        let children = &document.get_node(parent)?.children;
        if let Some(&position) = self.positions.get(&node) {
            return Some((parent, position));
        }
        if self.seen_parents.insert(parent) {
            // One-off getters need only a position scan, not a full index.
            let mut scan = children.iter();
            let position = scan.position(|&child| child == node);
            #[cfg(test)]
            super::tests::record_sibling_work(0, children.len() - scan.len());
            return Some((parent, position?));
        }
        // A second query promotes the whole parent. In a valid source forest,
        // every later child query hits positions above; no Indexed flag is needed.
        self.positions.reserve(children.len());
        for (position, &child) in children.iter().enumerate() {
            if document.get_node(child).is_some() && document.parent_of(child) == Some(parent) {
                self.positions.entry(child).or_insert(position);
            }
        }
        #[cfg(test)]
        super::tests::record_sibling_work(1, children.len());
        Some((parent, *self.positions.get(&node)?))
    }
}

#[cfg(test)]
mod tests;
