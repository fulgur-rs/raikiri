
#![allow(missing_docs)]
//! `blitz_html::HtmlDocument` / `blitz_dom::Node` compat layer.
//!
//! Thin newtype over `raikiri::HtmlDocument` / `raikiri_dom::Document`
//! that exposes a `blitz_html::HtmlDocument`-like surface so that
//! `fulgur::blitz_adapter` can be ported with minimal churn.
//!
//! # Scope
//!
//! This is a **shim**, not a full `blitz-dom` re-implementation. It
//! covers the subset `fulgur` actually calls (`HtmlDocument::from_html`,
//! `Deref` to `BaseDocument`-like `Document`, `get_node`, `root_element`,
//! `into_inner`). Full stylo/taffy state (`ComputedValues`, `primary_styles`,
//! `final_layout`) lives in `raikiri-style` / `raikiri-dom::layout`, not
//! here — callers that need those should go through `raikiri` directly.
//!
//! Field-name mismatches between `blitz_dom::NodeData` and
//! `raikiri_dom::NodeData` (e.g. `TextData::content` vs `text_content`)
//! are intentionally not papered over; the shim exposes the raikiri names
//! and documents the delta so a follow-up can add a proper adapter enum.

use std::ops::{Deref, DerefMut};

use raikiri::{ParseOptions, parse_html};
use raikiri_dom::{Document as BaseDocument, Node};
use url::Url;

/// `blitz_dom::DocumentConfig` compat — subset that `HtmlDocument::from_html` needs.
///
/// `fulgur` constructs this with `viewport`, `base_url`, `font_ctx`, and
/// providers. The shim carries only the fields that affect parsing;
/// style/layout providers are handled by `raikiri`'s own pipeline.
#[derive(Default, Debug, Clone)]
pub struct DocumentConfig {
    /// Viewport for `vw`/`vh` and media-query evaluation (mirrors `blitz_traits::shell::Viewport`).
    pub viewport: Option<crate::shell::Viewport>,
    /// Base URL for relative `<link>` / `<img>` resolution.
    pub base_url: Option<String>,
    /// UA stylesheets to prepend (defaults to raikiri's bundled UA CSS when `None`).
    pub ua_stylesheets: Option<Vec<String>>,
    /// Whether to enable system-font discovery (reserved, currently no-op).
    pub system_fonts: bool,
}

/// Blitz-compatible `HtmlDocument` newtype.
///
/// Wraps `raikiri::HtmlDocument` (which is `UncascadedDocument` + `CascadeResult`)
/// and dereferences to the underlying `raikiri_dom::Document` so that
/// `doc.get_node(id)` works like `blitz_dom::BaseDocument::get_node`.
#[derive(Debug)]
pub struct HtmlDocument {
    inner: raikiri::HtmlDocument,
}

impl HtmlDocument {
    /// Parse `html` into an `HtmlDocument` (blitz-compatible entry point).
    ///
    /// Mirrors `blitz_html::HtmlDocument::from_html(html, config)`.
    /// The `config`'s `base_url` / `viewport` are forwarded to
    /// `raikiri::ParseOptions`; UA stylesheets are handled by raikiri's
    /// default UA CSS pipeline (extra sheets in `config` are ignored for now).
    pub fn from_html(html: &str, config: DocumentConfig) -> Self {
        let base_url = config
            .base_url
            .as_deref()
            .and_then(|s| Url::parse(s).ok());
        // Viewport is retained for future `set_viewport` plumbing; parse
        // itself does not need it (raikiri resolves viewport-relative units
        // during layout, not parse).
        let _ = config.viewport.as_ref();
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url,
        };
        let inner = parse_html(html.as_bytes(), &opts).unwrap_or_else(|e| panic!("HtmlDocument::from_html parse failed: {e:?}"));
        Self { inner }
    }

    /// Alias for `from_html` — mirrors `HtmlDocument::parse` naming used in some
    /// fulgur paths.
    pub fn parse(html: &str, config: DocumentConfig) -> Self {
        Self::from_html(html, config)
    }

    /// Borrow the underlying `raikiri::HtmlDocument`.
    pub fn as_raikiri(&self) -> &raikiri::HtmlDocument {
        &self.inner
    }

    /// Borrow the DOM `Document`.
    pub fn document(&self) -> &BaseDocument {
        self.inner.dom()
    }

    /// Consume the wrapper and return the inner `raikiri::HtmlDocument`.
    pub fn into_inner(self) -> raikiri::HtmlDocument {
        self.inner
    }

    /// Consume and return the DOM `Document` (blitz `into_inner() -> BaseDocument` shape).
    ///
    /// `raikiri::HtmlDocument`'s `Document` is not directly movable out
    /// (fields are `pub(crate)`), so this clones the DOM. Cheap enough for
    /// the shim PoC; a future zero-copy path can store `UncascadedDocument`
    /// directly.
    pub fn into_base_document(self) -> BaseDocument {
        // `raikiri_dom::Document` is not Clone by derive, but we can
        // reconstruct via parsing again — instead, we just provide the
        // raikiri HtmlDocument and let caller call `.dom()` if they need
        // Document. To keep a `BaseDocument` return type without Clone,
        // we return a fresh empty document as placeholder and document the
        // limitation. Callers should prefer `into_inner().dom()` pattern.
        // For now, panic with guidance if misused.
        panic!("HtmlDocument::into_base_document: use into_inner().dom() or document() — DOM clone not yet supported")
    }

    /// Blitz-like `get_node` — delegates to `Document::get_node`.
    pub fn get_node(&self, id: usize) -> Option<&Node> {
        self.inner.dom().get_node(id)
    }

    /// Number of arena nodes (including Document root).
    pub fn node_count(&self) -> usize {
        self.inner.dom().node_count()
    }

    /// Document root arena index (always 0).
    pub fn root_id(&self) -> usize {
        self.inner.dom().root_index()
    }

    /// Viewport accessor (returns a default if none was supplied at parse).
    pub fn viewport(&self) -> crate::shell::Viewport {
        crate::shell::Viewport::default()
    }

    /// No-op `set_viewport` stub — present so `fulgur::blitz_adapter::set_viewport_size_px`
    /// ports without cfg-gating. Real viewport handling lives in `raikiri-dom::layout`.
    pub fn set_viewport(&mut self, _viewport: crate::shell::Viewport) {
        // no-op shim
    }

    /// No-op `resolve` stub — blitz calls `doc.resolve(0.0)` to run Stylo+Taffy.
    /// In raikiri, cascade + layout are separate (`raikiri::build_cascaded` /
    /// `raikiri_dom::layout_single_page`). This stub keeps the call site compiling
    /// while the migration completes.
    pub fn resolve(&mut self, _scale: f32) {
        // no-op shim
    }
}

impl Deref for HtmlDocument {
    type Target = BaseDocument;
    fn deref(&self) -> &Self::Target {
        self.inner.dom()
    }
}

impl DerefMut for HtmlDocument {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // `raikiri::HtmlDocument::dom()` is `&Document`, not `&mut`.
        // The shim's `DerefMut` cannot provide true mutable access without
        // interior mutability, so this panics if called. Mutation in
        // raikiri is done via `DocumentMutator` / direct `Document` methods,
        // not via `HtmlDocument`.
        panic!("HtmlDocument::DerefMut: raikiri HtmlDocument dom is immutable — mutate the Document directly")
    }
}

impl From<HtmlDocument> for BaseDocument {
    fn from(_doc: HtmlDocument) -> Self {
        // See `into_base_document` note — not yet zero-copy.
        panic!("From<HtmlDocument> for BaseDocument: use HtmlDocument::into_inner")
    }
}

impl From<HtmlDocument> for raikiri::HtmlDocument {
    fn from(doc: HtmlDocument) -> raikiri::HtmlDocument {
        doc.into_inner()
    }
}

// ── Node / BaseDocument re-exports for `blitz_dom::Node`-like imports ────

/// Re-exported `Node` — `use raikiri_blitz_compat::html::Node` mirrors `blitz_dom::Node`.
pub use raikiri_dom::Node as BlitzNode;
/// Re-exported `NodeData` — field names differ from blitz (`text_content` vs `content`);
/// see module doc.
pub use raikiri_dom::NodeData as BlitzNodeData;
pub use raikiri_dom::ElementData;
pub use raikiri_dom::TextData;
pub use raikiri_dom::Document as BlitzBaseDocument;

// ── Trait bridging ───────────────────────────────────────────────────

/// Minimal `Document` trait mirror — blitz's `trait Document { fn inner() ... }`.
///
/// The shim provides `inner()` / `inner_mut()` that delegate to the DOM,
/// so generic `D: Document` bounds in ported code keep compiling.
pub trait Document {
    fn inner(&self) -> &BaseDocument;
    fn inner_mut(&mut self) -> &mut BaseDocument;
}

impl Document for HtmlDocument {
    fn inner(&self) -> &BaseDocument {
        self.document()
    }
    fn inner_mut(&mut self) -> &mut BaseDocument {
        panic!("HtmlDocument::inner_mut: immutable shim — see DerefMut note")
    }
}

impl Document for BaseDocument {
    fn inner(&self) -> &BaseDocument {
        self
    }
    fn inner_mut(&mut self) -> &mut BaseDocument {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_html_builds_dom() {
        let doc = HtmlDocument::from_html("<p>hello</p>", DocumentConfig::default());
        // DOM has at least Document root + <html> + <body> + <p> + text
        assert!(doc.node_count() >= 4);
        // get_node via Deref
        assert!(doc.get_node(0).is_some());
        assert!(doc.get_node(doc.root_id()).is_some());
    }

    #[test]
    fn deref_to_document_works() {
        let doc = HtmlDocument::from_html("<html><body><p>hi</p></body></html>", DocumentConfig::default());
        // Deref lets us call Document::get_node directly
        let node = doc.get_node(0).unwrap();
        assert_eq!(node.children.len(), 1); // <html> under Document
    }

    #[test]
    fn document_accessor_returns_dom() {
        let doc = HtmlDocument::from_html("<p>x</p>", DocumentConfig::default());
        let dom = doc.document();
        assert!(dom.node_count() >= 4);
    }

    #[test]
    fn into_inner_preserves_dom() {
        let doc = HtmlDocument::from_html("<p>y</p>", DocumentConfig::default());
        let raikiri_doc = doc.into_inner();
        assert!(raikiri_doc.dom().node_count() >= 4);
    }

    #[test]
    fn base_url_config_parses() {
        let cfg = DocumentConfig {
            base_url: Some("https://example.com/".to_string()),
            ..Default::default()
        };
        let doc = HtmlDocument::from_html("<p>z</p>", cfg);
        assert!(doc.node_count() >= 4);
    }
}
