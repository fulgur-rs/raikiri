//! The embedder-provided side of the DOM runtime.
//!
//! The runtime binds JavaScript directly to a raikiri-dom [`Document`], but
//! owns neither that document nor style/layout. An embedder (a test runner, a
//! paginating renderer) implements [`DocumentHost`] to own the document
//! together with its stylesheets and layout state.

use raikiri_dom::Document;

/// `DOMRect` values in CSS pixels, relative to the initial containing block.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DomRect {
    /// Left edge.
    pub left: f64,
    /// Top edge.
    pub top: f64,
    /// Right edge.
    pub right: f64,
    /// Bottom edge.
    pub bottom: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// Layout geometry of one element's principal box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoxGeometry {
    /// The border box.
    pub border_box: DomRect,
}

/// A failure inside the embedder (layout, stylesheet loading, fragment
/// parsing). Scripts see it as an exception; the runtime also records it so
/// harnesses can report an engine error instead of a test failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError(pub String);

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HostError {}

/// Document ownership, style/layout, and HTML fragment parsing for a
/// [`super::DomRuntime`].
pub trait DocumentHost: 'static {
    /// The document scripts operate on.
    fn document(&self) -> &Document;
    /// Mutable access for DOM mutations performed by bindings.
    fn document_mut(&mut self) -> &mut Document;
    /// Bring style and layout up to date after DOM mutations.
    ///
    /// Called by the runtime only when a mutation happened since the last
    /// successful flush. Implementations re-derive author stylesheets from the
    /// connected document before cascading.
    fn flush(&mut self) -> Result<(), HostError>;
    /// Geometry of `node`'s principal box, or `None` when it has no box
    /// (for example `display: none`). Called after [`Self::flush`].
    fn box_geometry(&mut self, node: usize) -> Result<Option<BoxGeometry>, HostError>;
    /// Serialized computed value of `property` on `node`, or `None` when the
    /// property is not supported. Called after [`Self::flush`].
    fn computed_value(&mut self, node: usize, property: &str) -> Result<Option<String>, HostError>;
    /// Parse `markup` as an HTML fragment in the context of an element with
    /// the given local name and namespace. The fragment's nodes are the
    /// children of the returned document's root.
    fn parse_fragment(
        &mut self,
        context_tag: &str,
        context_ns: &str,
        markup: &str,
    ) -> Result<Document, HostError>;
}
