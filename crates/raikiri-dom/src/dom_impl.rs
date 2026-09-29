//! `raikiri_traits::Dom / Node / Element` and `raikiri_style::StyleDom /
//! StyleNode / StyleElement` implementations on Document.
//!
//! `NodeRef<'a>` and `ElementRef<'a>` are lightweight wrappers that borrow
//! the internal Document arena. GATs keep trait method return types stable.
//!
//! # Two trait families, one arena
//!
//! `raikiri_traits::Dom / Node / Element` is the neutral DOM abstraction shared by
//! raikiri-html / raikiri-paint / the raikiri umbrella. `raikiri_style::StyleDom
//! / StyleNode / StyleElement` is the CSS-engine abstraction that raikiri-style
//! maintains itself, following the Stylo pattern. The two are conceptually
//! almost identical, but raikiri-style does not depend on raikiri-traits.
//! Thus `impl StyleDom for Document` dispatches directly to the Document arena
//! rather than delegating to `raikiri_traits::Dom` (atomic decoupling that removes
//! the blanket compatibility path; see the header of crates/raikiri-style/src/style_dom.rs).
//! Bodies shared byte-identically across the two families
//! (as_element / text_content / is_in_document / tag_name /
//! inline_style_source / namespace_uri / attr / id) live once as private
//! inherent methods on NodeRef / ElementRef; both trait families delegate via
//! `self.foo()`. `kind()` is excluded — it projects onto
//! `NodeKind` vs `StyleNodeKind`.

use raikiri_style::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};
use raikiri_traits::{NodeId, NodeKind, QuirksMode};

use crate::document::Document;
use crate::node::Node;

/// Node reference borrowed from a Document arena.
pub struct NodeRef<'a> {
    doc: &'a Document,
    id: usize,
}

/// Element reference (a Node narrowed by type to kind == Element).
pub struct ElementRef<'a> {
    node: &'a Node,
}

/// Child NodeId iterator for `Dom::child_ids`.
pub struct ChildIter<'a>(core::slice::Iter<'a, usize>);

impl Iterator for ChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(|i| NodeId::new(i as u64))
    }
}

// Shared inherent method bodies for the two trait families below
// (see module doc header).

impl<'a> NodeRef<'a> {
    fn as_element(&self) -> Option<ElementRef<'_>> {
        let node = &self.doc.nodes[self.id];
        matches!(&node.data, crate::node::NodeData::Element(_)).then(|| ElementRef { node })
    }

    fn text_content(&self) -> Option<&str> {
        match &self.doc.nodes[self.id].data {
            crate::node::NodeData::Text(t) => Some(t.text_content.as_str()),
            _ => None,
        }
    }

    fn is_in_document(&self) -> bool {
        self.doc.nodes[self.id].is_in_document()
    }
}

impl<'a> ElementRef<'a> {
    fn tag_name(&self) -> &str {
        // ElementRef is a view returned after as_element() yields Some, so this must be
        // NodeData::Element (invariant); anything else would be equivalent to a panic.
        match &self.node.data {
            crate::node::NodeData::Element(e) => e.tag_name.as_str(),
            _ => "", // defensive: unreachable
        }
    }

    fn inline_style_source(&self) -> Option<&str> {
        match &self.node.data {
            crate::node::NodeData::Element(e) => {
                e.inline_style.as_deref().filter(|s| !s.is_empty())
            }
            _ => None,
        }
    }

    fn namespace_uri(&self) -> Option<&str> {
        match &self.node.data {
            crate::node::NodeData::Element(e) => e.namespace.as_deref(),
            _ => None,
        }
    }

    /// Null-namespace attribute lookup. Distinguishes "attribute absent"
    /// (`None`) from "attribute present with an empty value" (`Some("")`).
    ///
    /// CSS Selectors L4 attribute-presence (`[foo]`) and exact-value
    /// (`[foo=""]`) selectors, plus HTML boolean attributes (`disabled`,
    /// `open`, `hidden`, …), all depend on presence being observable
    /// independent of value — an element with `foo=""` still "has a `foo`
    /// attribute" per spec. Normalizing empty-to-absent here would make
    /// `[foo]` unable to match `<div foo="">` or `<dialog open>` (whose
    /// parsed attribute value is the empty string in both its bare and
    /// explicit-empty spellings).
    ///
    /// `id` has its own empty-is-absent contract (CSS Selectors L4 ID
    /// selectors don't match an empty ID token); that normalization lives in
    /// [`Self::id`] specifically, not here, so it doesn't leak into `[foo]` /
    /// `has_class` / future `[foo=bar]` matching that share this method.
    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            // Inherent-method resolution beats trait methods, so `self.` here
            // routes to the inherent `inline_style_source` above without UFCS.
            return self.inline_style_source();
        }
        match &self.node.data {
            crate::node::NodeData::Element(e) => e
                .attributes
                .iter()
                .find(|a| a.namespace.is_none() && a.local == local)
                .map(|a| a.value.as_str()),
            _ => None,
        }
    }

    /// `id` attribute value, with an empty `id=""` normalized to `None`.
    ///
    /// Narrow override of the empty-is-absent rule, scoped to `id()` alone
    /// (see [`Self::attr`]'s doc for why the general attribute lookup must
    /// NOT apply this normalization).
    fn id(&self) -> Option<&str> {
        self.attr("id").filter(|s| !s.is_empty())
    }
}

impl raikiri_traits::Dom for Document {
    type NodeRef<'a> = NodeRef<'a>;
    type ElementRef<'a> = ElementRef<'a>;
    type ChildIter<'a> = ChildIter<'a>;

    fn root_id(&self) -> NodeId {
        NodeId::new(self.root as u64)
    }

    fn node(&self, id: NodeId) -> Option<Self::NodeRef<'_>> {
        let idx = id.0 as usize;
        if idx < self.nodes.len() {
            Some(NodeRef { doc: self, id: idx })
        } else {
            None
        }
    }

    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_> {
        let idx = id.0 as usize;
        // Contract-align with `node()`: out-of-range NodeId → empty iter, not panic.
        // Guards consumer patterns that stash NodeId across document rebuilds,
        // including future blitz-compat integration.
        let slice = self
            .nodes
            .get(idx)
            .map(|n| n.children.as_slice())
            .unwrap_or(&[]);
        ChildIter(slice.iter())
    }

    fn node_count(&self) -> usize {
        // Trait-based view of Document::node_count(): the number of all arena nodes,
        // including detached nodes.
        Document::node_count(self)
    }
}

impl<'a> raikiri_traits::Node for NodeRef<'a> {
    type Element<'b>
        = ElementRef<'b>
    where
        Self: 'b;

    fn kind(&self) -> NodeKind {
        self.doc.nodes[self.id].kind()
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
        self.as_element()
    }

    fn text_content(&self) -> Option<&str> {
        self.text_content()
    }

    fn is_in_document(&self) -> bool {
        self.is_in_document()
    }
}

impl<'a> raikiri_traits::Element for ElementRef<'a> {
    fn tag_name(&self) -> &str {
        self.tag_name()
    }

    fn inline_style_source(&self) -> Option<&str> {
        self.inline_style_source()
    }

    fn namespace_uri(&self) -> Option<&str> {
        self.namespace_uri()
    }

    // NB: has_class() uses the default implementation of raikiri-traits::Element.
    // The default looks up the class through self.attr(...), so overriding attr()
    // also updates has_class (DRY and contract compliance). Unlike attr(), id()
    // has a normalization contract (empty value = absent), so override it separately.
    // (shared inherent `id()` above; see its doc).

    fn attr(&self, local: &str) -> Option<&str> {
        self.attr(local)
    }

    fn id(&self) -> Option<&str> {
        self.id()
    }
}

// ─────────────────────────────────────────────────────────────
// raikiri_style::{StyleDom, StyleNode, StyleElement} direct impls —
// Node/Element bodies via the shared inherent helpers above.
// ─────────────────────────────────────────────────────────────

/// Child `StyleNodeId` iterator for `StyleDom::child_ids`.
///
/// Wraps the arena `children: Vec<usize>` iterator and yields `StyleNodeId`
/// directly — no round-trip through `raikiri_traits::NodeId` (that would
/// re-couple raikiri-style's trait boundary to raikiri-traits identifiers).
pub struct StyleChildIter<'a>(core::slice::Iter<'a, usize>);

impl Iterator for StyleChildIter<'_> {
    type Item = StyleNodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(|i| StyleNodeId::new(i as u64))
    }
}

impl StyleDom for Document {
    type NodeRef<'a> = NodeRef<'a>;
    type ChildIter<'a> = StyleChildIter<'a>;

    fn root_id(&self) -> StyleNodeId {
        StyleNodeId::new(self.root as u64)
    }

    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>> {
        let idx = id.0 as usize;
        if idx < self.nodes.len() {
            Some(NodeRef { doc: self, id: idx })
        } else {
            None
        }
    }

    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        let idx = id.0 as usize;
        // Contract-align with `node()`: out-of-range → empty iter, not panic
        // (mirrors the `raikiri_traits::Dom` impl above).
        let slice = self
            .nodes
            .get(idx)
            .map(|n| n.children.as_slice())
            .unwrap_or(&[]);
        StyleChildIter(slice.iter())
    }

    fn node_count(&self) -> usize {
        Document::node_count(self)
    }

    fn quirks_mode(&self) -> StyleQuirksMode {
        convert_quirks_mode(Document::quirks_mode(self))
    }

    fn parent_id(&self, child: StyleNodeId) -> Option<StyleNodeId> {
        Document::parent_of(self, child.0 as usize).map(|p| StyleNodeId::new(p as u64))
    }
}

/// [`raikiri_traits::QuirksMode`] (raikiri-html's parse-time mirror of
/// html5ever's `QuirksMode`, carried on `Document` via
/// [`Document::set_quirks_mode`]) → [`StyleQuirksMode`] (raikiri-style's own
/// mirror, kept independent of raikiri-traits per this module's header).
/// Variant-for-variant 1:1 mapping — both enums model the same DOM Standard
/// 3-way quirks-mode state (<https://dom.spec.whatwg.org/#concept-document-quirks>).
/// Both enums are `#[non_exhaustive]`, so a wildcard arm is required for this
/// cross-crate match to compile; it falls back to `NoQuirks`, mirroring
/// `StyleDom::quirks_mode`'s own documented safe default for any future
/// `QuirksMode` variant this function doesn't yet know about.
fn convert_quirks_mode(mode: QuirksMode) -> StyleQuirksMode {
    match mode {
        QuirksMode::NoQuirks => StyleQuirksMode::NoQuirks,
        QuirksMode::LimitedQuirks => StyleQuirksMode::LimitedQuirks,
        QuirksMode::Quirks => StyleQuirksMode::Quirks,
        _ => StyleQuirksMode::NoQuirks,
    }
}

impl<'a> StyleNode for NodeRef<'a> {
    type Element<'b>
        = ElementRef<'b>
    where
        Self: 'b;

    fn kind(&self) -> StyleNodeKind {
        // NodeData variant → StyleNodeKind projection. Direct arena lookup —
        // no raikiri_traits::NodeKind bridge.
        //
        // Added Comment / ProcessingInstruction / DocumentFragment arms.
        // The cascade / rule-tree walkers only process Elements by contract
        // (crates/raikiri-style/src/cascade.rs / ruletree.rs), so the added kinds
        // are non-styling automatically. Matching kind() projections in both trait
        // families are part of the two-way invariant.
        match &self.doc.nodes[self.id].data {
            crate::node::NodeData::Element(_) => StyleNodeKind::Element,
            crate::node::NodeData::Text(_) => StyleNodeKind::Text,
            crate::node::NodeData::Document => StyleNodeKind::Document,
            crate::node::NodeData::Comment(_) => StyleNodeKind::Comment,
            crate::node::NodeData::ProcessingInstruction { .. } => {
                StyleNodeKind::ProcessingInstruction
            }
            crate::node::NodeData::DocumentFragment => StyleNodeKind::DocumentFragment,
        }
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
        self.as_element()
    }

    fn text_content(&self) -> Option<&str> {
        self.text_content()
    }

    fn is_in_document(&self) -> bool {
        self.is_in_document()
    }
}

impl<'a> StyleElement for ElementRef<'a> {
    fn tag_name(&self) -> &str {
        self.tag_name()
    }

    fn inline_style_source(&self) -> Option<&str> {
        self.inline_style_source()
    }

    fn namespace_uri(&self) -> Option<&str> {
        self.namespace_uri()
    }

    // NB: has_class() uses the default implementation of raikiri_style::StyleElement.
    // The default looks up the class through self.attr(...), so overriding attr()
    // also updates has_class (DRY and contract compliance). Unlike attr(), id()
    // has a normalization contract (empty value = absent), so override it separately.
    // (shared inherent `id()` above; see its doc).

    fn attr(&self, local: &str) -> Option<&str> {
        self.attr(local)
    }

    fn id(&self) -> Option<&str> {
        self.id()
    }
}

#[cfg(test)]
mod tests;
