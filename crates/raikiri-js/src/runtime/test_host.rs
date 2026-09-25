//! Deterministic [`DocumentHost`] for unit tests: a real raikiri-dom
//! document with fixed geometry and computed values.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use raikiri_dom::Document;

use super::host::{BoxGeometry, DocumentHost, DomRect, HostError};

pub(crate) struct StubHost {
    pub document: Document,
    pub flushes: Rc<Cell<usize>>,
    pub geometry: HashMap<usize, BoxGeometry>,
    pub computed: HashMap<(usize, String), String>,
    pub fail_flush: bool,
}

impl StubHost {
    /// `<html><head></head><body></body></html>` and returns (host, html, head, body).
    pub fn page() -> (Self, usize, usize, usize) {
        let mut document = Document::new();
        let root = document.root_index();
        let html = document.create_detached_element("html").unwrap();
        // `append_child` only accepts Element parents; the document root
        // takes its element child through the parser-level attach.
        document.attach_child(root, html);
        let head = document.create_detached_element("head").unwrap();
        document.append_child(html, head).unwrap();
        let body = document.create_detached_element("body").unwrap();
        document.append_child(html, body).unwrap();
        document.mark_in_document_flags();
        (
            Self {
                document,
                flushes: Rc::new(Cell::new(0)),
                geometry: HashMap::new(),
                computed: HashMap::new(),
                fail_flush: false,
            },
            html,
            head,
            body,
        )
    }

    #[allow(dead_code, reason = "used by geometry tests")]
    pub fn rect(height: f64) -> BoxGeometry {
        BoxGeometry {
            border_box: DomRect {
                left: 1.0,
                top: 2.0,
                right: 11.0,
                bottom: 2.0 + height,
                width: 10.0,
                height,
            },
        }
    }
}

impl DocumentHost for StubHost {
    fn document(&self) -> &Document {
        &self.document
    }
    fn document_mut(&mut self) -> &mut Document {
        &mut self.document
    }
    fn flush(&mut self) -> Result<(), HostError> {
        self.flushes.set(self.flushes.get() + 1);
        if self.fail_flush {
            Err(HostError("stub flush failure".into()))
        } else {
            Ok(())
        }
    }
    fn box_geometry(&mut self, node: usize) -> Result<Option<BoxGeometry>, HostError> {
        Ok(self.geometry.get(&node).copied())
    }
    fn computed_value(&mut self, node: usize, property: &str) -> Result<Option<String>, HostError> {
        Ok(self.computed.get(&(node, property.to_owned())).cloned())
    }
    fn parse_fragment(
        &mut self,
        _tag: &str,
        _ns: &str,
        markup: &str,
    ) -> Result<Document, HostError> {
        // Minimal fragment: a single text child, enough for innerHTML plumbing tests.
        let mut fragment = Document::new();
        let root = fragment.root_index();
        fragment.append_text(root, markup);
        Ok(fragment)
    }
}
