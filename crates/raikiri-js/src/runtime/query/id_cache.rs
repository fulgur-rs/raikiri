//! Revision-scoped ID lookups with bounded scalar results and source indices.
//!
//! Small working sets keep early-stopping walks and scalar memoization. Before
//! a root's scalar miss would evict the memo, index its actual IDs in preorder.
//! Complete indices answer arbitrary missing keys without retaining them;
//! partial indices use the scalar fallback for keys outside their prefix.
//!
//! Across all roots, indices retain at most 16,384 actual IDs, 1 MiB of UTF-8
//! key bytes and 128 root records. The scalar fallback separately retains at
//! most 128 results and 16 KiB of keys, hence at most 128 additional root maps.
//! Both layers skip keys over 1 KiB. These are key and entry budgets, excluding
//! hash-table metadata. Beyond an index budget, bounded fallback can still
//! walk the tree. Metadata contains scalar node IDs and owned keys, never JS
//! wrappers, and is released on revision change or runtime drop.

mod index;

use std::collections::HashMap;

use raikiri_dom::Document;
use raikiri_dom::document::NamedElementVersion;

use index::RootIndex;

const MAX_ENTRIES: usize = 128;
const MAX_KEY_BYTES: usize = 16 * 1024;
const MAX_SINGLE_KEY_BYTES: usize = 1024;
const MAX_INDEX_ENTRIES: usize = 16 * 1024;
const MAX_INDEX_KEY_BYTES: usize = 1024 * 1024;
const MAX_INDEX_ROOTS: usize = 128;

#[derive(Default)]
pub(crate) struct IdLookupCache {
    version: Option<NamedElementVersion>,
    roots: HashMap<usize, HashMap<String, Option<usize>>>,
    entries: usize,
    key_bytes: usize,
    indices: HashMap<usize, RootIndex>,
    index_entries: usize,
    index_key_bytes: usize,
}

impl IdLookupCache {
    fn clear_memo(&mut self) {
        self.roots = HashMap::new();
        self.entries = 0;
        self.key_bytes = 0;
    }

    fn clear(&mut self) {
        self.clear_memo();
        self.indices = HashMap::new();
        self.index_entries = 0;
        self.index_key_bytes = 0;
    }

    fn memo_would_overflow(&self, id: &str) -> bool {
        self.entries == MAX_ENTRIES || self.key_bytes + id.len() > MAX_KEY_BYTES
    }

    fn promote(&mut self, document: &Document, root: usize) {
        let index = RootIndex::build(
            document,
            root,
            MAX_INDEX_ENTRIES - self.index_entries,
            MAX_INDEX_KEY_BYTES - self.index_key_bytes,
        );
        self.index_entries += index.ids.len();
        self.index_key_bytes += index.key_bytes;
        if index.complete
            && let Some(memo) = self.roots.remove(&root)
        {
            self.entries -= memo.len();
            self.key_bytes -= memo.keys().map(String::len).sum::<usize>();
        }
        // Keep partial and empty indices as well: otherwise each later miss
        // could retry the same exhausted budget and rebuild the same prefix.
        self.indices.insert(root, index);
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
        if id.len() > MAX_SINGLE_KEY_BYTES {
            return find_uncached(document, root, id);
        }
        if let Some(index) = self.indices.get(&root) {
            if let Some(&element) = index.ids.get(id) {
                return Some(element);
            }
            if index.complete {
                return None;
            }
        }
        if let Some(&result) = self.roots.get(&root).and_then(|entries| entries.get(id)) {
            return result;
        }
        if !self.indices.contains_key(&root)
            && self.indices.len() < MAX_INDEX_ROOTS
            && self.roots.contains_key(&root)
            && self.memo_would_overflow(id)
        {
            // A first query on a new root keeps its early stop even when other
            // roots fill the memo. Promote before doing the miss's normal walk.
            self.promote(document, root);
            let index = &self.indices[&root];
            if let Some(&element) = index.ids.get(id) {
                return Some(element);
            }
            if index.complete {
                return None;
            }
        }
        let found = find_uncached(document, root, id);
        if self.memo_would_overflow(id) {
            self.clear_memo();
        }
        // Fallback churn must not discard the independent source-ID indices.
        self.roots
            .entry(root)
            .or_default()
            .insert(id.to_owned(), found);
        self.entries += 1;
        self.key_bytes += id.len();
        found
    }
}

fn find_uncached(document: &Document, root: usize, id: &str) -> Option<usize> {
    walk(document, root, |element| {
        (document.element_attribute(element, "id") == Some(id)).then_some(element)
    })
}

fn walk<T>(
    document: &Document,
    root: usize,
    mut visit: impl FnMut(usize) -> Option<T>,
) -> Option<T> {
    #[cfg(test)]
    let mut visits = 0;
    let found = super::find_in_tree(document, root, |element, _| {
        #[cfg(test)]
        {
            visits += 1;
        }
        visit(element)
    });
    #[cfg(test)]
    super::tests::record_id_visits(visits);
    found
}

#[cfg(test)]
mod tests;
