//! Stylesheet IDs of the last observed native document revision.
//!
//! Storage is O(last observed sheet count), kept until the next list query
//! or runtime drop. Rebuilding replaces the previous Vec and releases its
//! capacity. Only native IDs are kept, so cached entries cannot root wrappers.

use raikiri_dom::Document;
use raikiri_dom::document::StylesheetVersion;

#[derive(Default)]
pub(crate) struct SheetList {
    version: Option<StylesheetVersion>,
    nodes: Vec<usize>,
}

impl SheetList {
    pub(super) fn nodes(&mut self, document: &Document) -> &[usize] {
        if self.version.as_ref() != Some(document.stylesheet_version()) {
            self.nodes = super::stylesheet_nodes(document);
            self.version = Some(document.stylesheet_version().clone());
        }
        &self.nodes
    }
}

#[cfg(test)]
mod tests;
