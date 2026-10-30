//! A bounded preorder prefix of actual element IDs, independent of query keys.

use std::collections::HashMap;

use raikiri_dom::Document;

use super::{MAX_SINGLE_KEY_BYTES, walk};

pub(super) struct RootIndex {
    pub(super) ids: HashMap<String, usize>,
    pub(super) key_bytes: usize,
    pub(super) complete: bool,
}

impl RootIndex {
    pub(super) fn build(
        document: &Document,
        root: usize,
        available_entries: usize,
        available_key_bytes: usize,
    ) -> Self {
        let mut index = Self {
            ids: HashMap::new(),
            key_bytes: 0,
            complete: true,
        };
        walk(document, root, |element| {
            let id = document.element_attribute(element, "id")?;
            if id.is_empty() || id.len() > MAX_SINGLE_KEY_BYTES || index.ids.contains_key(id) {
                return None;
            }
            // Only a new eligible ID that cannot fit makes the prefix partial.
            // Duplicate IDs must keep their first occurrence, even at quota.
            if index.ids.len() == available_entries
                || id.len() > available_key_bytes - index.key_bytes
            {
                index.complete = false;
                return Some(());
            }
            index.ids.insert(id.to_owned(), element);
            index.key_bytes += id.len();
            None
        });
        index
    }
}

#[cfg(test)]
mod tests;
