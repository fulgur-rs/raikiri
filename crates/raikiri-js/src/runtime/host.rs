//! The embedder-provided side of the DOM runtime.
//!
//! The runtime binds JavaScript directly to a raikiri-dom [`Document`], but
//! owns neither that document nor style/layout. An embedder (a test runner, a
//! paginating renderer) implements [`DocumentHost`] to own the document
//! together with its stylesheets and layout state.

use std::any::Any;

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

/// Computed value of the `position` property (CSS Positioned Layout 3 §2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PositionKind {
    /// `static`.
    #[default]
    Static,
    /// `relative`.
    Relative,
    /// `absolute`.
    Absolute,
    /// `fixed`.
    Fixed,
    /// `sticky`.
    Sticky,
}

/// Layout geometry of one element's principal box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoxGeometry {
    /// The border box.
    pub border_box: DomRect,
    /// The padding box (border box minus border widths).
    pub padding_box: DomRect,
    /// Scrollable overflow width measured from the padding box origin
    /// (at least the padding box width).
    pub scroll_width: f64,
    /// Scrollable overflow height measured from the padding box origin
    /// (at least the padding box height).
    pub scroll_height: f64,
    /// Computed `position`.
    pub position: PositionKind,
    /// Whether the box is a non-atomic inline box (`display: inline`),
    /// whose `client*` metrics are zero (CSSOM View §6).
    pub is_inline: bool,
}

/// A failure inside the embedder (layout, stylesheet loading, fragment
/// parsing). Scripts see it as an exception; the runtime also records it so
/// harnesses can report an engine error instead of a test failure. This is
/// the behavior for every [`DocumentHost`] method except
/// [`DocumentHost::fetch_script`], whose own doc comment explains why its
/// `HostError` is handled differently.
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
///
/// The downcast hooks ([`DocumentHost::as_any`], [`DocumentHost::as_any_mut`],
/// [`DocumentHost::into_any`]) let an embedder recover its concrete host from
/// [`super::DomRuntime::into_host`]'s boxed trait object, either through the
/// [`dyn DocumentHost`](DocumentHost#impl-dyn-DocumentHost) downcast helpers
/// or directly with [`super::DomRuntime::try_into_host`].
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
    ///
    /// Contract: the returned root's children must never include a Document
    /// node. The `innerHTML` setter hands the returned root to
    /// `Document::replace_children_from`, which panics on a Document node
    /// found in the child list, so a host that returns one turns the setter
    /// into a runtime panic rather than a script-visible exception.
    fn parse_fragment(
        &mut self,
        context_tag: &str,
        context_ns: &str,
        markup: &str,
    ) -> Result<Document, HostError>;
    /// The document's own URL, already a valid absolute URL string (the
    /// serialization the WHATWG URL Standard would produce), or `None` when
    /// the embedder has no URL for it -- `window.location` and
    /// `document.URL`/`documentURI` then read as `about:blank`.
    ///
    /// A `String` rather than a parsed URL type: the `url` crate is not
    /// among this crate's own dependencies, and its callers only ever read
    /// this back as components of an already-valid string, never construct
    /// or validate one -- see `super::window`'s own narrow component
    /// extraction.
    fn document_url(&self) -> Option<String> {
        None
    }
    /// Fetch a classic script's source from its resolved `src` URL. The
    /// default implementation always fails, matching a host with no script
    /// loading of its own (an embedder that only ever runs inline scripts
    /// through [`super::DomRuntime::evaluate`] never needs to override
    /// this).
    ///
    /// Unlike every other [`DocumentHost`] method, this one's [`HostError`]
    /// is never turned into a thrown script exception and never counted in
    /// [`super::RunReport::host_failures`]: a fetch failure becomes a
    /// [`super::RunReport::fetch_errors`] entry plus a trusted `error` event
    /// fired at the `<script>` element, and the script itself is never
    /// evaluated -- there is nothing running yet for it to be thrown into.
    fn fetch_script(&mut self, _url: &str) -> Result<String, HostError> {
        Err(HostError("script fetching is not supported".into()))
    }
    /// Type-erased shared access to the concrete host, for downcasting a
    /// boxed [`DocumentHost`] back to the embedder's own type (see
    /// [`super::DomRuntime::try_into_host`]). Implement as `self`.
    fn as_any(&self) -> &dyn Any;
    /// Type-erased exclusive access to the concrete host, for downcasting a
    /// boxed [`DocumentHost`] back to the embedder's own type. Implement as
    /// `self`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Owned type-erased concrete host, for downcasting a boxed
    /// [`DocumentHost`] without borrowing. Implement as `self`: the box
    /// coerces into a [`std::any::Any`] box for every sized host type.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

impl dyn DocumentHost {
    /// Borrow the boxed host as the concrete type it was created with, or
    /// `None` when it holds a different host type.
    pub fn downcast_ref<T: DocumentHost>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// Exclusively borrow the boxed host as the concrete type it was created
    /// with, or `None` when it holds a different host type.
    pub fn downcast_mut<T: DocumentHost>(&mut self) -> Option<&mut T> {
        self.as_any_mut().downcast_mut::<T>()
    }

    /// Unbox the host as the concrete type it was created with, or return
    /// the boxed trait object untouched when it holds a different host type.
    pub fn downcast<T: DocumentHost>(self: Box<Self>) -> Result<Box<T>, Box<Self>> {
        if self.as_any().is::<T>() {
            let any: Box<dyn Any> = self.into_any();
            any.downcast::<T>().map_err(|_| {
                // cov:ignore: the `is` check above rules a type mismatch out,
                // so the `Any` downcast cannot fail here.
                unreachable!("DocumentHost type check passed but Any downcast failed")
            })
        } else {
            Err(self)
        }
    }
}
