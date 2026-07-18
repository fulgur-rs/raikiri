//! Style-owned DOM abstraction — cleanroom decoupling of raikiri-style from
//! raikiri-traits (Phase A of raikiri-spike-3ps).
//!
//! # Why a style-owned trait surface
//!
//! Stylo's archetype is `TDocument` / `TElement` / `TNode` living inside the
//! style crate, with individual DOM implementations (Blitz's `blitz-dom`,
//! Servo's `script`) providing the impls. raikiri mirrors that shape here so:
//!
//! - raikiri-style stops importing from raikiri-traits at the trait surface.
//! - raikiri-dom (Phase B, coord scope) can implement `StyleDom` directly and
//!   the workspace can drop the `raikiri-style → raikiri-traits` Cargo edge.
//! - Downstream code that used to write `raikiri_style::cascade(&doc, …)`
//!   against a `Document: raikiri_traits::Dom` still compiles unchanged during
//!   Phase A thanks to blanket compat impls below.
//!
//! # Naming: `StyleDom` vs reusing `Dom`
//!
//! We use `StyleDom` / `StyleElement` / `StyleNode` instead of reusing `Dom` /
//! `Element` / `Node`. Rationale:
//!
//! - Downstream code often uses both crates. `raikiri_traits::Dom` and
//!   `raikiri_style::Dom` living in the same file would create trait-selection
//!   ambiguity (`some_doc.root_id()` — which trait's method?). `StyleDom`
//!   disambiguates textually.
//! - Consumers importing `raikiri_style::*` are unlikely to be surprised —
//!   Stylo-flavoured names (`TDocument` → `StyleDom`) are the established
//!   pattern in the CSS-engine ecosystem.
//!
//! # `CascadeError`
//!
//! For Phase A we keep the raikiri-traits `CascadeError` variant and re-export
//! it from here. That keeps `raikiri_style::cascade()` returning the same
//! error identity umbrella code catches, and confines the raikiri-traits
//! reference to this compat module. Decoupling `CascadeError` itself is a
//! separate concern (it is an error taxonomy, not a DOM abstraction).

// Re-export the shared error identity. Deliberately routed through this compat
// module so `cascade.rs` does not need `use raikiri_traits::…` (Phase A grep
// invariant: `raikiri_traits` only appears here + Cargo.toml).
pub use raikiri_traits::CascadeError;

/// Stable identifier for DOM nodes inside a `StyleDom`.
///
/// Layout-compatible with `raikiri_traits::NodeId` on purpose: the blanket
/// compat impl converts between them by unwrapping the inner `u64`. Consumers
/// treat this as an opaque handle — cascade indexes `computed[id.0 as usize]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StyleNodeId(pub u64);

impl StyleNodeId {
    /// Construct a `StyleNodeId` from a raw `u64` identifier.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// DOM node kind — Element / Text / Document root.
///
/// `#[non_exhaustive]` so Comment / CDATA / ProcessingInstruction can be added
/// later without breakage (mirroring raikiri-traits' shape).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleNodeKind {
    /// HTML / XML element (has `tag_name`).
    Element,
    /// Character-data node.
    Text,
    /// Document root (virtual node at arena index 0 by contract).
    Document,
}

/// DOM tree abstraction consumed by raikiri-style's cascade / rule-tree walk.
///
/// GAT-based — non-object-safe — generic dispatch (`fn walk<D: StyleDom>`) is
/// the intended usage.
///
/// # Contract
///
/// - `root_id()` returns the Document node (usually `StyleNodeId(0)`).
/// - `node(id)` returns `Some(…)` iff `id.0 < node_count() as u64`.
/// - `child_ids(id)` returns an empty iterator for invalid `id` (mirroring
///   `node()`'s `None`).
/// - `node_count()` is the arena capacity, including detached / unreachable
///   nodes. Cascade pre-allocates `Vec<ComputedValues>` at this size.
pub trait StyleDom {
    /// Borrowed node reference.
    type NodeRef<'a>: StyleNode<'a>
    where
        Self: 'a;
    /// Borrowed element reference.
    type ElementRef<'a>: StyleElement<'a>
    where
        Self: 'a;
    /// Iterator over child node identifiers.
    type ChildIter<'a>: Iterator<Item = StyleNodeId>
    where
        Self: 'a;

    /// Document root node.
    fn root_id(&self) -> StyleNodeId;

    /// Node by identifier; `None` if out of range.
    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>>;

    /// Direct children of `id`; empty iterator for invalid ids.
    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_>;

    /// Total arena size (including detached / unreachable nodes).
    ///
    /// Default `0` is a safe fallback for shell implementations; real DOMs
    /// (raikiri-dom, test_dom) override with the true arena length.
    fn node_count(&self) -> usize {
        0
    }
}

/// Node reference (borrowed lifetime `'a`).
pub trait StyleNode<'a> {
    /// Element downcast reference type.
    type Element<'b>: StyleElement<'b>
    where
        Self: 'b;

    /// Node kind (Element / Text / Document).
    fn kind(&self) -> StyleNodeKind;

    /// Element downcast — `Some` iff `kind() == Element`.
    fn as_element(&self) -> Option<Self::Element<'_>>;

    /// Character data — `Some` iff `kind() == Text`.
    fn text_content(&self) -> Option<&str>;

    /// Whether this node is part of the flat tree.
    ///
    /// `<template>` descendants and other inert subtrees return `false`;
    /// raikiri-style traversals (cascade / rule-tree walk) skip them.
    ///
    /// Default `true` matches Blitz's `TElement::is_in_document → true`
    /// posture: unaware node implementations opt into "everything visible"
    /// rather than silently dropping to `false`.
    fn is_in_document(&self) -> bool {
        true
    }
}

/// Element reference (borrowed lifetime `'a`).
///
/// Attribute lookup covers only null-namespace attrs (namespaced attrs like
/// `xlink:href` are out of scope for M1).
pub trait StyleElement<'a> {
    /// Tag name (`"p"`, `"div"`, …).
    fn tag_name(&self) -> &str;

    /// Raw `style="…"` value; `None` if unset or empty.
    ///
    /// cascade parses this string with cssparser to derive inline
    /// declarations.
    fn inline_style_source(&self) -> Option<&str> {
        None
    }

    /// Namespace URI (`Some("http://www.w3.org/2000/svg")` for SVG etc).
    /// HTML default namespace returns `None` (fast path).
    fn namespace_uri(&self) -> Option<&str> {
        None
    }

    /// `id` attribute value (empty `id=""` returns `None`).
    ///
    /// Default delegates to `attr("id")` so overriding `attr` alone keeps
    /// `id()` consistent.
    fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    /// Whether `class` attribute contains the given token.
    ///
    /// HTML-spec ASCII whitespace split (space / tab / LF / CR / FF).
    /// Empty query always `false`.
    fn has_class(&self, class: &str) -> bool {
        if class.is_empty() {
            return false;
        }
        self.attr("class").is_some_and(|value| {
            value
                .split([' ', '\t', '\n', '\r', '\x0C'])
                .any(|token| token == class)
        })
    }

    /// Null-namespace attribute lookup. Empty string is normalised to `None`.
    ///
    /// Default handles `"style"` by delegating to
    /// [`Self::inline_style_source`]; overrides must preserve that contract.
    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            self.inline_style_source()
        } else {
            None
        }
    }
}

// ─────────────────────────────────────────────────────────────
// Blanket compat impls (Phase A only)
//
// TODO(raikiri-spike-3ps Phase B): remove these once raikiri-dom implements
// `StyleDom` / `StyleElement` / `StyleNode` directly. The removal happens
// atomically with dropping the `raikiri-traits` dependency from
// `raikiri-style/Cargo.toml`. Until then, every type that impls
// `raikiri_traits::Dom` (raikiri-dom's `Document`, sink adapters, etc.)
// automatically satisfies `StyleDom` through the blanket below.
// ─────────────────────────────────────────────────────────────

/// Convert a raikiri-traits `NodeId` into a `StyleNodeId` (used by the
/// blanket `child_ids` iterator adapter).
fn nid_to_style(n: raikiri_traits::NodeId) -> StyleNodeId {
    StyleNodeId(n.0)
}

/// Convert a raikiri-traits `NodeKind` into a `StyleNodeKind`.
///
/// Both enums are `#[non_exhaustive]` with the same variant set today. If
/// raikiri-traits adds a new variant (Comment, CDATA, …) this arm forces us
/// to mirror it in `StyleNodeKind` — fail-loud rather than silently
/// misclassify.
fn kind_to_style(k: raikiri_traits::NodeKind) -> StyleNodeKind {
    match k {
        raikiri_traits::NodeKind::Element => StyleNodeKind::Element,
        raikiri_traits::NodeKind::Text => StyleNodeKind::Text,
        raikiri_traits::NodeKind::Document => StyleNodeKind::Document,
        // `#[non_exhaustive]` requires a wildcard; keep it fail-loud so a new
        // upstream variant does not silently collapse to Element.
        _ => panic!(
            "raikiri_traits::NodeKind gained a new variant not mirrored in \
             StyleNodeKind — extend the compat blanket in style_dom.rs"
        ),
    }
}

impl<T> StyleDom for T
where
    T: raikiri_traits::Dom + ?Sized,
{
    type NodeRef<'a>
        = T::NodeRef<'a>
    where
        Self: 'a;
    type ElementRef<'a>
        = T::ElementRef<'a>
    where
        Self: 'a;
    type ChildIter<'a>
        = core::iter::Map<T::ChildIter<'a>, fn(raikiri_traits::NodeId) -> StyleNodeId>
    where
        Self: 'a;

    fn root_id(&self) -> StyleNodeId {
        // UFCS everywhere: `self.root_id()` would resolve back to this very
        // `StyleDom::root_id` impl and infinite-recurse.
        nid_to_style(raikiri_traits::Dom::root_id(self))
    }

    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>> {
        raikiri_traits::Dom::node(self, raikiri_traits::NodeId(id.0))
    }

    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        raikiri_traits::Dom::child_ids(self, raikiri_traits::NodeId(id.0))
            .map(nid_to_style as fn(raikiri_traits::NodeId) -> StyleNodeId)
    }

    fn node_count(&self) -> usize {
        raikiri_traits::Dom::node_count(self)
    }
}

impl<'a, N> StyleNode<'a> for N
where
    N: raikiri_traits::Node<'a>,
{
    type Element<'b>
        = N::Element<'b>
    where
        Self: 'b;

    fn kind(&self) -> StyleNodeKind {
        kind_to_style(raikiri_traits::Node::kind(self))
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
        raikiri_traits::Node::as_element(self)
    }

    fn text_content(&self) -> Option<&str> {
        raikiri_traits::Node::text_content(self)
    }

    fn is_in_document(&self) -> bool {
        raikiri_traits::Node::is_in_document(self)
    }
}

impl<'a, E> StyleElement<'a> for E
where
    E: raikiri_traits::Element<'a>,
{
    fn tag_name(&self) -> &str {
        raikiri_traits::Element::tag_name(self)
    }

    fn inline_style_source(&self) -> Option<&str> {
        raikiri_traits::Element::inline_style_source(self)
    }

    fn namespace_uri(&self) -> Option<&str> {
        raikiri_traits::Element::namespace_uri(self)
    }

    fn id(&self) -> Option<&str> {
        raikiri_traits::Element::id(self)
    }

    fn has_class(&self, class: &str) -> bool {
        raikiri_traits::Element::has_class(self, class)
    }

    fn attr(&self, local: &str) -> Option<&str> {
        raikiri_traits::Element::attr(self, local)
    }
}
