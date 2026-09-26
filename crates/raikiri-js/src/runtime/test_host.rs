//! Deterministic [`DocumentHost`] for unit tests: a real raikiri-dom
//! document with fixed geometry and computed values.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use raikiri_dom::Document;

use super::host::{BoxGeometry, DocumentHost, DomRect, HostError, PositionKind};

pub(crate) struct StubHost {
    pub document: Document,
    pub flushes: Rc<Cell<usize>>,
    pub geometry: HashMap<usize, BoxGeometry>,
    pub computed: HashMap<(usize, String), String>,
    pub fail_flush: bool,
    pub fail_geometry: bool,
    pub fail_computed: bool,
}

impl StubHost {
    /// `<html><head></head><body></body></html>` and returns (host, html, head, body).
    pub fn page() -> (Self, usize, usize, usize) {
        let mut document = Document::new();
        let root = document.root_index();
        let html = document.create_detached_element("html").unwrap();
        // The document root takes its element child through the
        // parser-level attach, mirroring how a real HTML parse wires the
        // root `<html>` element (`append_child` would also accept a
        // Document parent now, but this stub predates that and there is no
        // move-from-elsewhere semantics to exercise here).
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
                fail_geometry: false,
                fail_computed: false,
            },
            html,
            head,
            body,
        )
    }

    /// A 10px-wide static box at (1, 2) with the given height and no
    /// borders: its padding box is its border box, and nothing overflows it.
    pub fn rect(height: f64) -> BoxGeometry {
        let border = Self::dom_rect(1.0, 2.0, 10.0, height);
        Self::boxed(border, border, PositionKind::Static)
    }

    /// A `DomRect` from its origin and size.
    pub fn dom_rect(left: f64, top: f64, width: f64, height: f64) -> DomRect {
        DomRect {
            left,
            top,
            right: left + width,
            bottom: top + height,
            width,
            height,
        }
    }

    /// Geometry of a non-inline box with explicit border and padding boxes
    /// and `position`; the scroll extent is the padding box size (no
    /// overflow). Tests adjust the returned fields for anything else.
    pub fn boxed(border: DomRect, padding: DomRect, position: PositionKind) -> BoxGeometry {
        BoxGeometry {
            border_box: border,
            padding_box: padding,
            scroll_width: padding.width,
            scroll_height: padding.height,
            position,
            is_inline: false,
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
        if self.fail_geometry {
            return Err(HostError("stub geometry failure".into()));
        }
        Ok(self.geometry.get(&node).copied())
    }
    fn computed_value(&mut self, node: usize, property: &str) -> Result<Option<String>, HostError> {
        if self.fail_computed {
            return Err(HostError("stub computed style failure".into()));
        }
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
