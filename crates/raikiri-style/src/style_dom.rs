//! Style-owned DOM abstraction — cleanroom decoupling of raikiri-style from
//! raikiri-traits (a two-part decoupling — Phase A introduced this trait
//! surface, Phase B dropped the Cargo dependency on raikiri-traits entirely).
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

/// Document-mode context: HTML5 quirks mode. Mirror of
/// `raikiri_traits::QuirksMode` for the style-owned trait surface (Phase B
/// decoupling) — same rationale as
/// [`StyleNodeKind`] mirroring `raikiri_traits::NodeKind` just below:
/// raikiri-style does not depend on raikiri-traits (see this module's
/// header), so it carries its own copy of the 3-way state rather than
/// reaching across that dropped Cargo edge.
///
/// Consumed by [`StyleDom::quirks_mode`], which [`mod@crate::cascade`]'s
/// id/class selector matching reads to decide whether to ASCII-case-fold
/// (CSS Selectors L4 quirks-mode case-insensitivity).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StyleQuirksMode {
    /// Standards mode (`<!DOCTYPE html>` explicit, or no quirks-mode trigger
    /// matched).
    #[default]
    NoQuirks,
    /// Limited quirks mode ("almost standards mode"). CSS Selectors L4's
    /// id/class case-insensitivity applies only to
    /// [`StyleQuirksMode::Quirks`] — this variant is
    /// spec-distinct from it (DOM Standard
    /// <https://dom.spec.whatwg.org/#concept-document-quirks>, verbatim: "A
    /// document is said to be in no-quirks mode if its mode is 'no-quirks',
    /// **quirks mode** if its mode is 'quirks', and **limited-quirks mode**
    /// if its mode is 'limited-quirks'" — three separate named dfns).
    LimitedQuirks,
    /// Full quirks mode (missing / obsolete DOCTYPE).
    Quirks,
}

/// DOM node kind — mirror of `raikiri_traits::NodeKind` for the style-owned
/// trait surface (Phase B decoupling).
///
/// 後から `Comment` / `ProcessingInstruction` /
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
    /// 後から追加。
    Comment,
    /// Processing instruction node (`<?target data?>`)。cascade は skip する。
    /// 後から追加。
    ProcessingInstruction,
    /// Document fragment root (`<template>` contents 等)。detached subtree の
    /// virtual root、Document root から reachable でない。cascade は
    /// `is_in_document()` gate で skip する。後から追加。
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

    /// Document-mode context (HTML5 quirks mode) for this whole document.
    ///
    /// Default [`StyleQuirksMode::NoQuirks`] is a safe fallback for shell
    /// implementations (mirrors [`Self::node_count`]'s `0` default) — DOM
    /// implementations that already track parse-time quirks-mode detection
    /// (e.g. `raikiri_traits::QuirksMode`, set by raikiri-html's sink and
    /// carried through `UncascadedDocument`) should override this to report
    /// the true value. [`mod@crate::cascade`]'s id/class selector matching
    /// reads this to decide ASCII-case-folding (CSS Selectors L4).
    ///
    /// `raikiri-dom::Document` carries a real quirks-mode field
    /// (`Document::set_quirks_mode` / `Document::quirks_mode`, populated by
    /// raikiri-html's parse sink), and `impl StyleDom for Document`
    /// (`crates/raikiri-dom/src/dom_impl.rs`) overrides this method to
    /// report it — so real parsed HTML documents reach cascade with their
    /// actual doctype-derived quirks mode.
    fn quirks_mode(&self) -> StyleQuirksMode {
        StyleQuirksMode::NoQuirks
    }
}

/// DOM node abstraction — kind dispatch and common API.
///
/// **Lifetime elision**: method signatures do not
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
/// `xlink:href` are currently out of scope).
///
/// **Lifetime elision**: method signatures do not
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
    /// HTML-spec ASCII whitespace split (space / tab / LF / CR / FF), exact
    /// (case-sensitive) per-token comparison. Empty query always `false`.
    /// See [`Self::has_class_ascii_case_insensitive`] for the quirks-mode
    /// counterpart (CSS Selectors L4 class-html).
    fn has_class(&self, class: &str) -> bool {
        if class.is_empty() {
            return false;
        }
        self.attr("class")
            .is_some_and(|value| class_token_matches(value, class, false))
    }

    /// Whether `class` attribute contains the given token, matched ASCII
    /// case-insensitively (CSS Selectors L4
    /// <https://www.w3.org/TR/selectors-4/#class-html>, verbatim: "When
    /// matching against a document which is in quirks mode, class names must
    /// be matched ASCII case-insensitively; class selectors are otherwise
    /// case-sensitive").
    ///
    /// Same HTML-spec ASCII whitespace tokenisation as [`Self::has_class`] —
    /// only the per-token comparison differs (`eq_ignore_ascii_case` instead
    /// of `==`). Default delegates to `attr("class")` exactly like
    /// `has_class`, so overriding `attr` alone keeps both consistent.
    fn has_class_ascii_case_insensitive(&self, class: &str) -> bool {
        if class.is_empty() {
            return false;
        }
        self.attr("class")
            .is_some_and(|value| class_token_matches(value, class, true))
    }

    /// Null-namespace attribute lookup. Empty string is normalised to `None`.
    ///
    /// Default handles `"style"` by delegating to
    /// [`Self::inline_style_source`]; overrides must preserve that contract.
    ///
    /// **Known gap vs. CSS Selectors L4**: the
    /// attribute-presence selector form `[foo]`
    /// (<https://www.w3.org/TR/selectors-4/#attribute-selectors>) is defined
    /// to match on attribute *presence* alone, independent of value — an
    /// element with `foo=""` still "has a `foo` attribute" per spec and must
    /// match `[foo]`. This trait's `attr()` contract collapses `foo=""` into
    /// `None`, identically to `foo` being wholly absent (both the mock
    /// (`test_dom.rs`) and the real DOM impl
    /// (`raikiri-dom::dom_impl::ElementRef::attr`) honor this uniformly), so
    /// `Component::AttributeInNoNamespaceExists` matching
    /// (`cascade.rs::compound_matches`), which is built directly on
    /// `elem.attr(...).is_some()`, cannot observe the distinction: `[foo]`
    /// will not match `<div foo="">`.
    ///
    /// Same root cause, same blast radius: the exact-value form `[foo=""]`
    /// is affected identically, and for the same reason. Per spec `[foo=""]`
    /// should match an element carrying `foo=""` (attribute value compares
    /// equal to the empty string), but `Component::AttributeInNoNamespace`'s
    /// `compound_matches` arm (`cascade.rs`, the `elem.attr(local_name)
    /// => None => false` branch) already sees `None` for `foo=""` — it
    /// cannot distinguish "value is empty" from "attribute absent" any more
    /// than the `Exists` arm above can, so `[foo=""]` will not match
    /// `<div foo="">` either.
    ///
    /// There is no other `StyleElement` method that exposes raw
    /// presence-independent-of-value attribute information, so this cannot
    /// be worked around from within `raikiri-style` alone. Accepted as a
    /// permanent simplification rather than fixed, because fixing it
    /// would require changing this trait's contract (e.g. splitting out a
    /// `has_attr()` that distinguishes absent from present-but-empty, or
    /// widening `attr()`'s return type) — a `StyleElement` signature change
    /// that crosses into DOM-impl crate territory (`raikiri-dom`'s
    /// `impl StyleElement for ElementRef` in `dom_impl.rs`, which owns the
    /// filtering) and is out of scope for a `raikiri-style`-only change. No
    /// known real-world content in this repo's test corpus currently relies
    /// on presence-with-empty-value matching.
    ///
    /// The decision to accept this spec divergence as a permanent
    /// baseline (rather than fix it immediately) reflects a deliberate
    /// accept/reject call — "intentional stricter" divergence (this trait's
    /// `attr()` contract collapses `foo=""` into absent, which makes
    /// matching stricter than spec requires, not looser) — this is an
    /// accepted-baseline call, not a
    /// bug being silently tolerated. Regression-pinned by
    /// `cascade::tests::attribute_exists_selector_does_not_match_empty_value_attr`
    /// (the `[foo]` form) and
    /// `cascade::tests::attribute_exact_match_selector_does_not_match_empty_value_attr`
    /// (the `[foo=""]` form).
    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            self.inline_style_source()
        } else {
            None
        }
    }
}

/// Shared class-token search for [`StyleElement::has_class`] /
/// [`StyleElement::has_class_ascii_case_insensitive`] — HTML-spec ASCII
/// whitespace split, with the per-token comparison as the only thing that
/// differs between the two callers (`ascii_case_insensitive` selects
/// `eq_ignore_ascii_case` vs `==`).
fn class_token_matches(attr_value: &str, class: &str, ascii_case_insensitive: bool) -> bool {
    attr_value
        .split([' ', '\t', '\n', '\r', '\x0C'])
        .any(|token| {
            if ascii_case_insensitive {
                token.eq_ignore_ascii_case(class)
            } else {
                token == class
            }
        })
}
