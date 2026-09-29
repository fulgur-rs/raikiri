//! DOM abstraction trait + identifier newtypes.
//!
//! `Dom` / `Element` / `Node` will gain finalized associated types and methods.
//! For now, only the trait shells exist; their implementations will be
//! co-designed with raikiri-dom.

use smol_str::SmolStr;

/// String identifier for GCPM fragments / running templates / named strings /
/// target-* references.
///
/// A SmolStr newtype enables inline optimization. GCPM will use it extensively.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(pub SmolStr);

impl Symbol {
    /// Construct from any string type.
    pub fn new(s: impl Into<SmolStr>) -> Self {
        Self(s.into())
    }

    /// Borrow inner str.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// Stable identifier for DOM nodes across a document.
///
/// Referenced by `RenderWarning.node_id` and similar fields. raikiri-dom
/// projects its own arena node index into a u64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u64);

impl NodeId {
    /// Construct a node identifier from a raw u64.
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

/// DOM node kind (Element / Text / Document root / Comment /
/// ProcessingInstruction / DocumentFragment).
///
/// Corresponds to the kind field in raikiri-dom's node arena.
/// `Comment` / `ProcessingInstruction` / `DocumentFragment` add the three
/// NodeTypes from WHATWG DOM §4 relevant to paged-media rendering.
/// `#[non_exhaustive]` makes the change non-breaking (existing callers
/// match with a wildcard arm or `matches!(_, Element)`,
/// so they are unaffected).
///
/// Two-way invariant ([`Node::kind`] / [`Node::as_element`]):
/// `kind() == NodeKind::Element` iff `as_element().is_some()`. All added
/// `Comment` / `ProcessingInstruction` / `DocumentFragment` variants
/// all return `None` from `as_element()`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// HTML / XML element (has a tag_name).
    Element,
    /// Character data node.
    Text,
    /// Document root (virtual node at arena index 0).
    Document,
    /// HTML / XML comment node (`<!-- ... -->`). Holds character data
    /// but is not an Element (`as_element() == None`). Cascade / paint / layout
    /// traversals typically skip it via `is_in_document()`, but it remains
    /// available to DOM mutation APIs.
    Comment,
    /// Processing instruction node (`<?target data?>`; rare in HTML,
    /// valid in XML / XHTML). Holds target + data.
    ProcessingInstruction,
    /// Document fragment root (the contents fragment root of `<template>`
    /// or the virtual root of a detached subtree made by createDocumentFragment).
    /// Remains detached in the arena and is not reachable from the Document root
    /// (`is_in_document() == false` after mark_in_document_flags).
    /// Added to fix the shape of `<template>` fragment roots (formerly
    /// an Element with the pseudo-tag `"#document-fragment"`).
    DocumentFragment,
}

/// DOM tree abstraction. raikiri-dom / raikiri-style / raikiri-paint / raikiri
/// (umbrella) consumes this generic navigation interface.
///
/// **Object safety**: This trait has a GAT (`type NodeRef<'a>`) and is not object-safe.
/// Currently assumes generic dispatch (`fn walk<D: Dom>(dom: &D)`). If dyn dispatch
/// becomes necessary (for example, future runtime abstraction through blitz-compat),
/// provide a separate erased wrapper trait.
pub trait Dom {
    /// Node reference (borrowed) type.
    type NodeRef<'a>: Node
    where
        Self: 'a;
    /// Element reference (borrowed) type.
    type ElementRef<'a>: Element
    where
        Self: 'a;
    /// Child ID iterator type.
    type ChildIter<'a>: Iterator<Item = NodeId>
    where
        Self: 'a;

    /// Identifier of the Document root node. Implementations usually refer to the
    /// Document-kind node at arena index 0.
    fn root_id(&self) -> NodeId;

    /// Node reference for `id`, or `None` if out of range.
    fn node(&self, id: NodeId) -> Option<Self::NodeRef<'_>>;

    /// Iterate over direct children of `id`; return an empty iterator for an
    /// out-of-range NodeId (symmetric with [`node`](Self::node) returning `None`).
    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_>;

    /// Total nodes in the arena (including the Document root and detached /
    /// unreachable nodes).
    ///
    /// Used by traversal code such as cascade to preallocate `Vec<T>`.
    /// **Contract**: Every `NodeId(0..node_count as u64)` returns `Some` from
    /// [`node`](Self::node). Conversely, `id.0 >= node_count as u64` returns
    /// `None`.
    ///
    /// The default implementation returns `0`: a safe fallback for existing callers.
    /// New implementations of `Dom` should override it. Existing raikiri-dom
    /// and raikiri-style TestDoc implementations already do.
    fn node_count(&self) -> usize {
        0
    }
}

/// DOM node abstraction providing dispatch by kind and a shared API.
///
/// **Lifetime elision**: This was formerly `Node<'a>`, but the lifetime
/// was dropped because method signatures did not use `'a`.
/// The lifetime of the borrowed Node value is instead expressed by `'a`
/// in `Dom::NodeRef<'a>`; the trait parameter is unnecessary. The GAT
/// `Element<'b>` remains the type of a borrowed element reference.
pub trait Node {
    /// Element reference type used for downcasting.
    type Element<'b>: Element
    where
        Self: 'b;

    /// Kind of this node.
    fn kind(&self) -> NodeKind;

    /// Return an Element reference if the kind is Element; otherwise
    /// (Text / Document / Comment / ProcessingInstruction / DocumentFragment)
    /// return `None` — pinned by the two-way invariant (`kind() == NodeKind::Element` iff
    /// `as_element().is_some()`).
    fn as_element(&self) -> Option<Self::Element<'_>>;

    /// Return character data if the kind is Text; otherwise (Element / Document /
    /// Comment / ProcessingInstruction / DocumentFragment) return `None`.
    /// Even when Comment / PI have character data, this method returns only the Text
    /// variant (explicitly distinguishing kinds).
    fn text_content(&self) -> Option<&str>;

    /// Whether this Node belongs to the flat tree. Descendants of `<template>`
    /// elements return `false`; nodes reachable from the Document root through
    /// flat-tree-parent edges return `true`.
    ///
    /// Traversals (cascade / paint / stylesheet extraction) use this predicate
    /// to skip inert subtrees consistently. Do not scatter individual tag_name
    /// checks (`== "template"`, etc.) across traversals: that hides the concept
    /// and risks omissions when shadow DOM is added.
    ///
    /// # Default impl
    ///
    /// The default always returns `true`, a safe fallback so Node implementations
    /// lacking this concept (such as unimplemented test nodes) are not silently
    /// dropped (like blitz `stylo.rs` `TElement::is_in_document -> true`).
    /// raikiri-dom `NodeRef` overrides this with the actual flag.
    ///
    /// ```
    /// use raikiri_traits::Node;
    /// # struct DummyNode;
    /// # struct DummyElem;
    /// # impl raikiri_traits::Element for DummyElem { fn tag_name(&self) -> &str { "" } }
    /// # impl Node for DummyNode {
    /// #   type Element<'b> = DummyElem where Self: 'b;
    /// #   fn kind(&self) -> raikiri_traits::NodeKind { raikiri_traits::NodeKind::Document }
    /// #   fn as_element(&self) -> Option<Self::Element<'_>> { None }
    /// #   fn text_content(&self) -> Option<&str> { None }
    /// # }
    /// // The default implementation always returns true; unaware implementations need not override.
    /// let n = DummyNode;
    /// assert!(n.is_in_document());
    /// ```
    fn is_in_document(&self) -> bool {
        true
    }
}

/// DOM element abstraction.
///
/// Added `inline_style_source` / `namespace_uri` / `id` / `has_class` / `attr`.
/// Attribute lookup is limited to null-namespace attributes (namespaced
/// attributes such as xlink:href are deferred).
///
/// **Lifetime elision**: This was formerly `Element<'a>`, but the lifetime
/// was dropped because method signatures did not use `'a`.
/// The lifetime of the borrowed Element value is instead expressed by
/// `'a` in `Dom::ElementRef<'a>`; the trait parameter is unnecessary.
pub trait Element {
    /// HTML / XML tag name (for example, `"p"` or `"div"`).
    fn tag_name(&self) -> &str;

    /// Return the raw string of the HTML `style="..."` attribute.
    /// Return `None` when unset or empty (`style=""`).
    ///
    /// raikiri-style::cascade consumes the string through cssparser's
    /// declaration-list parser. Kept as a separate method rather than an
    /// alias for `self.attr("style")`.
    ///
    /// The default implementation returns `None`; Node kinds without styles
    /// and implementations without support need not override it.
    fn inline_style_source(&self) -> Option<&str> {
        None
    }

    /// Namespace URI of the Element (for example, `"http://www.w3.org/2000/svg"`).
    /// Elements in the default HTML namespace return `None` (optimized path).
    ///
    /// The raikiri-html sink copies html5ever's `QualName.ns` (interned URI)
    /// into SmolStr and stores it in raikiri-dom::Node.
    fn namespace_uri(&self) -> Option<&str> {
        None
    }

    /// Value of the `id` attribute. Empty `id=""` returns `None`: a CSS
    /// Selectors L4 ID selector (`#foo`) cannot match an empty ID token.
    /// This empty-is-absent normalization belongs only to `id()`;
    /// it is not the general contract of [`Element::attr`] (`attr()` tracks
    /// attribute presence independently of value and returns `Some("")`
    /// for an empty value; see the [`Element::attr`] docs). Multiple tokens
    /// are invalid by spec, but the raw value is returned without tokenization.
    ///
    /// The default delegates directly to [`Element::attr`]`("id")`, so
    /// implementations overriding only `attr()` do not automatically satisfy
    /// the empty-is-absent rule. Implementations that must treat empty
    /// `id=""` as `None` (such as `raikiri-dom::dom_impl::ElementRef`)
    /// must override `id()` separately.
    fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    /// Whether the `class` attribute (space-separated) contains the given token.
    /// Split on ASCII whitespace (space, tab, LF, CR, FF) per the HTML spec.
    /// Return `false` if `class` is absent or empty, or the token is absent.
    ///
    /// The default tokenizes [`Element::attr`]`("class")`.
    /// Implementations need only override attr for has_class to follow.
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

    /// Look up a null-namespace attribute by local name.
    /// **Track attribute presence independently of value**: Return `None` only
    /// when the attribute is unset; return `Some("")` when it exists but is empty.
    /// CSS Selectors L4 attribute-presence selectors (`[foo]`) and
    /// exact-value selectors (`[foo=""]`), as well as HTML boolean attributes
    /// (`disabled` / `open` / `hidden`, etc.), rely on presence separately
    /// from value; never treat an empty value as absence.
    /// (The empty-is-absent normalization of `id` belongs to [`Element::id`]
    /// and does not apply to general `attr()`; see the [`Element::id`] docs.)
    ///
    ///
    /// For `style`, the return value matches [`Element::inline_style_source`]
    /// (both views refer to the same source). `inline_style_source` itself
    /// has the separate contract that empty `style=""` returns `None`
    /// (see [`Element::inline_style_source`] docs). Thus `style` is an exception to general `attr()`
    /// presence/value separation and always delegates directly to
    /// `inline_style_source()`. The default implementation returns
    /// [`inline_style_source`](Element::inline_style_source) for "style" only;
    /// otherwise it always returns `None` (no attribute storage in the
    /// default implementation). Implementations overriding `attr` should
    /// also delegate "style" to `self.inline_style_source()`.
    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            self.inline_style_source()
        } else {
            None
        }
    }
}

/// HTML5 quirks mode. Set by raikiri-html and used by cascade through
/// UncascadedDocument. A raikiri-side mirror of html5ever `QuirksMode`
/// (independent implementation: no html5ever dependency in the trait layer).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QuirksMode {
    /// Standards mode (explicit `<!DOCTYPE html>` or no quirks-mode trigger).
    #[default]
    NoQuirks,
    /// Limited quirks mode.
    LimitedQuirks,
    /// Full quirks mode (missing / obsolete DOCTYPE).
    Quirks,
}

// ─────────────────────────────────────────────────────────────
// StylesheetKind — kind of stylesheet associated with a Document.
// ─────────────────────────────────────────────────────────────

/// Kind of stylesheet associated with a Document.
///
/// Nominal tag reflecting the CSS Cascading L4 §6.2 origin concept
/// at the DOM level. Mapped to `raikiri_style::Origin` during the
/// cascade phase (raikiri umbrella crate).
///
/// Initially only `UserAgent` and `Author` existed. Adding the `User` variant
/// allows CSS supplied through the consumer's `extra_stylesheets` to have
/// an independent user origin. The old implementation temporarily mixed it
/// with `Author` (listed as a Non-goal in the design doc), which risked
/// classifying future real author-origin stylesheets (`<link rel=stylesheet>`,
/// etc.) on this route as user-origin. The variant prevents that regression.
///
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StylesheetKind {
    /// User Agent origin (bundled minimal UA CSS, etc.). Weakest in the cascade,
    /// but strongest for `!important` (CSS Cascading L4 §6.3 Importance
    /// <https://www.w3.org/TR/css-cascade-4/#importance> reverses origin
    /// precedence; see §6.2 for the definition of origin:
    /// <https://www.w3.org/TR/css-cascade-4/#cascading-origins>).
    UserAgent,
    /// User origin (CSS Cascading L4 §6.2
    /// <https://www.w3.org/TR/css-cascade-4/#cascading-origins>). The consumer's
    /// CSS supplied through `ParseOptions::extra_stylesheets` is tagged here
    /// (`raikiri-html/src/parse.rs`; separated from `Author`).
    User,
    /// Author origin (HTML `<style>` elements, `<link rel=stylesheet>`, etc.).
    Author,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Verify that `Symbol`'s `Ord` / `PartialOrd` implementation inherits
    /// lexicographic order from SmolStr (= &str). This unit contract is needed
    /// before target-* / fragment-id logic relies on the deterministic
    /// iteration order of `BTreeMap<Symbol, _>`.
    #[test]
    fn symbol_ord_matches_str_lexicographic() {
        // 2-element comparison: "a" < "b" (str lexicographic).
        let a = Symbol::from("a");
        let b = Symbol::from("b");
        assert!(a < b);
        assert_eq!(a.cmp(&b), std::cmp::Ordering::Less);

        // BTreeSet iterates in sorted order regardless of insertion order.
        let mut set = BTreeSet::new();
        set.insert(Symbol::from("charlie"));
        set.insert(Symbol::from("alpha"));
        set.insert(Symbol::from("bravo"));
        let collected: Vec<&str> = set.iter().map(Symbol::as_str).collect();
        assert_eq!(collected, ["alpha", "bravo", "charlie"]);
    }

    /// Check that the added `NodeKind` variants `Comment` /
    /// `ProcessingInstruction` / `DocumentFragment` can be distinguished by
    /// pattern matching and compared as unequal to `Element` with PartialEq
    /// (a trait-side constraint of the two-way invariant).
    #[test]
    fn node_kind_variants_are_distinct_and_matchable() {
        for kind in [
            NodeKind::Element,
            NodeKind::Text,
            NodeKind::Document,
            NodeKind::Comment,
            NodeKind::ProcessingInstruction,
            NodeKind::DocumentFragment,
        ] {
            // All three added variants differ from Element under PartialEq.
            if !matches!(kind, NodeKind::Element) {
                assert_ne!(
                    kind,
                    NodeKind::Element,
                    "{kind:?} must not compare equal to Element"
                );
            }
        }
        // Discriminant consistency: equal variants compare equal.
        assert_eq!(NodeKind::Comment, NodeKind::Comment);
        assert_eq!(
            NodeKind::ProcessingInstruction,
            NodeKind::ProcessingInstruction
        );
        assert_eq!(NodeKind::DocumentFragment, NodeKind::DocumentFragment);
    }
}
