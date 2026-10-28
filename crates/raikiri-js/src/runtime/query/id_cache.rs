//! Bounded scalar ID lookup results, scoped to a root and native revision.
//!
//! Repeated keys use average O(1) hash access with respect to tree size.
//! New keys retain the existing descendant walk. At most 128 entries and
//! 16 KiB of key bytes are retained; keys over 1 KiB bypass storage. Metadata
//! holds no wrappers and remains until a later lookup or runtime drop.

use std::collections::HashMap;

use raikiri_dom::Document;
use raikiri_dom::document::NamedElementVersion;

const MAX_ENTRIES: usize = 128;
const MAX_KEY_BYTES: usize = 16 * 1024;
const MAX_SINGLE_KEY_BYTES: usize = 1024;

#[derive(Default)]
pub(crate) struct IdLookupCache {
    version: Option<NamedElementVersion>,
    roots: HashMap<usize, HashMap<String, Option<usize>>>,
    entries: usize,
    key_bytes: usize,
}

impl IdLookupCache {
    fn clear(&mut self) {
        self.roots = HashMap::new();
        self.entries = 0;
        self.key_bytes = 0;
    }

    pub(super) fn find(&mut self, document: &Document, root: usize, id: &str) -> Option<usize> {
        if self.version.as_ref() != Some(document.named_element_version()) {
            // Even empty/invalid queries release the previous document's keys.
            self.clear();
            self.version = Some(document.named_element_version().clone());
        }
        if id.is_empty() || document.get_node(root).is_none() {
            return None;
        }
        if let Some(&result) = self.roots.get(&root).and_then(|entries| entries.get(id)) {
            return result;
        }
        #[cfg(test)]
        let mut visits = 0;
        let found = super::find_in_tree(document, root, |element, _| {
            #[cfg(test)]
            {
                visits += 1;
            }
            (document.element_attribute(element, "id") == Some(id)).then_some(element)
        });
        #[cfg(test)]
        super::tests::record_id_visits(visits);
        if id.len() <= MAX_SINGLE_KEY_BYTES {
            if self.entries == MAX_ENTRIES || self.key_bytes + id.len() > MAX_KEY_BYTES {
                self.clear();
            }
            // Allocate a root map only when actually retaining a result, so
            // root-map count is bounded by the global entry count as well.
            self.roots
                .entry(root)
                .or_default()
                .insert(id.to_owned(), found);
            self.entries += 1;
            self.key_bytes += id.len();
        }
        found
    }
}

#[cfg(test)]
mod tests;
