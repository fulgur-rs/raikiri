//! Style-owned DOM abstraction — cleanroom decoupling of raikiri-style from
//! raikiri-traits (raikiri-spike-3ps Phase A + 94e Phase B).
//!
//! # Why a style-owned trait surface
//!
//! Stylo's archetype is `TDocument` / `TElement` / `TNode` living inside the
//! style crate, with individual DOM implementations (Blitz's `blitz-dom`,
//! Servo's `script`) providing the impls. raikiri mirrors that shape here —
//! raikiri-style stops depending on raikiri-traits entirely (Cargo edge
//! dropped in Phase B), and raikiri-dom implements `StyleDom` /
//! `StyleElement` / `StyleNode` directly on its `Document` / `NodeRef` /
//! `ElementRef` (see `crates/raikiri-dom/src/dom_impl.rs`).
//!
//! # Naming: `StyleDom` vs reusing `Dom`
//!
//! We use `StyleDom` / `StyleElement` / `StyleNode` instead of reusing `Dom`
//! / `Element` / `Node`. Rationale:
//!
//! - Downstream code often uses both `raikiri-style` and `raikiri-traits`.
//!   `Dom` / `Element` / `Node` names in both crates would create
//!   trait-selection ambiguity (`some_doc.root_id()` — which trait's
//!   method?). `StyleDom` disambiguates textually.
//! - Consumers importing `raikiri_style::*` are unlikely to be surprised —
//!   Stylo-flavoured names (`TDocument` → `StyleDom`) are the established
//!   pattern in the CSS-engine ecosystem.
//!
//! # `CascadeError`
//!
//! `CascadeError` lives in [`crate::error`] (moved from `raikiri-traits`
//! in Phase B). raikiri-traits re-exports it back so
//! `RenderError::Cascade(CascadeError)` stays stable at the umbrella surface.

/// Stable identifier for DOM nodes inside a `StyleDom`.
///
/// Opaque handle — cascade indexes `computed[id.0 as usize]`. The inner
/// `u64` layout is chosen to match raikiri-dom's arena index scheme; the
/// raikiri-dom `StyleDom` impl converts between its arena `usize` and
/// `StyleNodeId` at the trait boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StyleNodeId(pub u64);

impl StyleNodeId {
    /// Construct a `StyleNodeId` from a raw `u64` identifier.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// DOM node kind — mirror of `raikiri_traits::NodeKind` for the style-owned
/// trait surface (Phase B decoupling, raikiri-spike-3ps + 94e).
///
/// raikiri-spike-84y (Sprint 20) で `Comment` / `ProcessingInstruction` /
/// `DocumentFragment` を追加。`#[non_exhaustive]` により変更は non-breaking。
///
/// Two-way invariant ([`StyleNode::kind`] / [`StyleNode::as_element`]):
/// `kind() == StyleNodeKind::Element` iff `as_element().is_some()`。追加された
/// 3 variant はすべて `as_element() == None`。cascade / rule-tree walk は
/// `StyleNodeKind::Element` のみ処理し、他 kind は skip する契約なので、新
/// variant は自動的に non-styling (cascade は 触らない) となる。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleNodeKind {
    /// HTML / XML element (has `tag_name`).
    Element,
    /// Character-data node.
    Text,
    /// Document root (virtual node at arena index 0 by contract).
    Document,
    /// Comment node (`<!-- ... -->`)。cascade は skip する (Element でない)。
    /// raikiri-spike-84y で追加。
    Comment,
    /// Processing instruction node (`<?target data?>`)。cascade は skip する。
    /// raikiri-spike-84y で追加。
    ProcessingInstruction,
    /// Document fragment root (`<template>` contents 等)。detached subtree の
    /// virtual root、Document root から reachable でない。cascade は
    /// `is_in_document()` gate で skip する。raikiri-spike-84y で追加。
    DocumentFragment,
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
    type NodeRef<'a>: StyleNode
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

/// DOM node abstraction — kind dispatch and common API.
///
/// **Lifetime elision** (raikiri-spike-2ng): method signatures do not
/// reference `'a`; the borrowed node value's lifetime is expressed via
/// `StyleDom::NodeRef<'a>`, so the trait itself needs no lifetime
/// parameter. GAT `Element<'b>` remains as the borrowed element reference
/// type.
pub trait StyleNode {
    /// Element downcast reference type.
    type Element<'b>: StyleElement
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

/// DOM element abstraction.
///
/// Attribute lookup covers only null-namespace attrs (namespaced attrs like
/// `xlink:href` are out of scope for M1).
///
/// **Lifetime elision** (raikiri-spike-2ng): method signatures do not
/// reference `'a`; the borrowed element value's lifetime is expressed via
/// `StyleDom::NodeRef<'a>` and `StyleNode::Element<'b>`, so the trait
/// itself needs no lifetime parameter.
pub trait StyleElement {
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
