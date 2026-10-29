//! Source-node synchronization for the HTML tree sink's snapshots.

use super::Document;

impl Document {
    /// Reuse a previous raw-parser snapshot's source arena.
    ///
    /// Only the tree sink may use this operation: its canonical document has
    /// no document-level stylesheets, fonts, images or layout state to copy.
    /// `changed` must include every previously existing source node changed
    /// by parser callbacks. Consumers may populate namespace, attributes and
    /// reachability, as tree-sink assembly does, but must not alter source
    /// topology, text or authored styles. Newly appended nodes and the root are
    /// cloned automatically. All document metadata and generated boxes are
    /// reset, and reachability flags must be recomputed before use.
    /// Returns the fresh shell and the number of source node clone operations.
    /// Reallocation of the arena can additionally relocate existing storage.
    #[doc(hidden)]
    pub fn snapshot_parser_nodes_into(
        &self,
        previous: Document,
        changed: &[usize],
    ) -> (Document, usize) {
        let mut nodes = previous.nodes.into_source();
        nodes.truncate(self.nodes.len());
        let old_len = nodes.len();
        let mut changed = changed.to_vec();
        changed.push(self.root);
        changed.sort_unstable();
        changed.dedup();
        let mut copied = 0;
        for id in changed.into_iter().filter(|&id| id < old_len) {
            nodes[id].clone_from(&self.nodes[id]);
            copied += 1;
        }
        copied += self.nodes.len() - old_len;
        nodes.extend(self.nodes.iter().skip(old_len).cloned());
        let mut document = Document::new();
        document.nodes = nodes.into();
        document.root = self.root;
        document.flags_dirty = true;
        document.layout_dirty = true;
        (document, copied)
    }
}
