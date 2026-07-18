//! `raikiri_traits::Dom / Node / Element` および `raikiri_style::StyleDom /
//! StyleNode / StyleElement` implementations on Document.
//!
//! `NodeRef<'a>` と `ElementRef<'a>` は Document の内部 arena を borrow する
//! 軽量 wrapper。GAT 経由で trait method の返り値型を安定させる。
//!
//! # Two trait families, one arena
//!
//! `raikiri_traits::Dom / Node / Element` は raikiri-html / raikiri-paint /
//! raikiri umbrella が共有する neutral DOM abstraction。`raikiri_style::StyleDom
//! / StyleNode / StyleElement` は raikiri-style が Stylo pattern に沿って自前
//! で持つ CSS-engine 向け abstraction (raikiri-spike-3ps + 94e)。両者は概念
//! 上ほぼ相似形だが、raikiri-style は raikiri-traits に依存しないため、
//! `impl StyleDom for Document` は `raikiri_traits::Dom` に delegate せず
//! Document arena に直接 dispatch する (blanket compat 経路を排除した atomic
//! decoupling — 詳細は crates/raikiri-style/src/style_dom.rs のヘッダ参照)。

use raikiri_style::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};
use raikiri_traits::{NodeId, NodeKind};

use crate::document::Document;
use crate::node::Node;

/// Node reference borrowed from a Document arena.
pub struct NodeRef<'a> {
    doc: &'a Document,
    id: usize,
}

/// Element reference (kind == Element の Node を型で narrow したもの)。
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
        // Guards Consumer patterns that stash NodeId across document rebuilds
        // (raikiri-spike-ajy, M6 blitz-compat integration).
        let slice = self
            .nodes
            .get(idx)
            .map(|n| n.children.as_slice())
            .unwrap_or(&[]);
        ChildIter(slice.iter())
    }

    fn node_count(&self) -> usize {
        // Document::node_count() の trait 経由 view (raikiri-spike-37c, roborev
        // job 293 M1 finding 対応)。arena 全 node の数 (detached を含む)。
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

impl<'a> raikiri_traits::Element for ElementRef<'a> {
    fn tag_name(&self) -> &str {
        // ElementRef は as_element() が Some を返した後の view なので必ず
        // NodeData::Element (invariant)、それ以外は panic 相当。
        match &self.node.data {
            crate::node::NodeData::Element(e) => e.tag_name.as_str(),
            _ => "", // defensive: 到達しない
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

    // NB: id() / has_class() は raikiri-traits::Element の default impl を
    // 使用。default が self.attr(...) 経由で lookup するため、この impl は
    // attr() だけ override すれば id/has_class も追従する (DRY / 契約準拠)。

    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            // UFCS: `ElementRef` also implements `raikiri_style::StyleElement`,
            // so plain `self.inline_style_source()` is ambiguous (raikiri-spike-94e
            // Phase B — both trait families coexist on the same nominal type).
            return raikiri_traits::Element::inline_style_source(self);
        }
        match &self.node.data {
            crate::node::NodeData::Element(e) => e
                .attributes
                .iter()
                .find(|a| a.local == local)
                .map(|a| a.value.as_str())
                .filter(|s| !s.is_empty()),
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// raikiri_style::{StyleDom, StyleNode, StyleElement} direct impls
// (raikiri-spike-94e Phase B)
//
// These delegate to the Document arena directly, NOT via `raikiri_traits::Dom`.
// The atomic decoupling in Phase B removed the blanket compat
// (`impl<T: raikiri_traits::Dom> StyleDom for T`) that previously bridged the
// two trait families, so this file provides the concrete implementations.
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
}

impl<'a> StyleNode<'a> for NodeRef<'a> {
    type Element<'b>
        = ElementRef<'b>
    where
        Self: 'b;

    fn kind(&self) -> StyleNodeKind {
        // NodeData variant → StyleNodeKind projection. Direct arena lookup —
        // no raikiri_traits::NodeKind bridge.
        match &self.doc.nodes[self.id].data {
            crate::node::NodeData::Element(_) => StyleNodeKind::Element,
            crate::node::NodeData::Text(_) => StyleNodeKind::Text,
            crate::node::NodeData::Document => StyleNodeKind::Document,
        }
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
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

impl<'a> StyleElement<'a> for ElementRef<'a> {
    fn tag_name(&self) -> &str {
        match &self.node.data {
            crate::node::NodeData::Element(e) => e.tag_name.as_str(),
            _ => "", // defensive: 到達しない (ElementRef invariant)
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

    // NB: id() / has_class() は raikiri_style::StyleElement の default impl を
    // 使用。default が self.attr(...) 経由で lookup するため、この impl は
    // attr() だけ override すれば id/has_class も追従する (DRY / 契約準拠)。

    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            // UFCS to disambiguate from the sibling `raikiri_traits::Element`
            // impl on `ElementRef` (see the twin note in that impl above).
            return StyleElement::inline_style_source(self);
        }
        match &self.node.data {
            crate::node::NodeData::Element(e) => e
                .attributes
                .iter()
                .find(|a| a.local == local)
                .map(|a| a.value.as_str())
                .filter(|s| !s.is_empty()),
            _ => None,
        }
    }
}
