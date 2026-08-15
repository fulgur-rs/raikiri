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
//! で持つ CSS-engine 向け abstraction。両者は概念
//! 上ほぼ相似形だが、raikiri-style は raikiri-traits に依存しないため、
//! `impl StyleDom for Document` は `raikiri_traits::Dom` に delegate せず
//! Document arena に直接 dispatch する (blanket compat 経路を排除した atomic
//! decoupling — 詳細は crates/raikiri-style/src/style_dom.rs のヘッダ参照)。
//! Bodies shared byte-identically across the two families
//! (as_element / text_content / is_in_document / tag_name /
//! inline_style_source / namespace_uri / attr) live once as private inherent
//! methods on NodeRef / ElementRef; both trait families delegate via
//! `self.foo()`. `kind()` is excluded — it projects onto
//! `NodeKind` vs `StyleNodeKind`.

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
                .find(|a| a.local == local)
                .map(|a| a.value.as_str())
                .filter(|s| !s.is_empty()),
            _ => None,
        }
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
        // Document::node_count() の trait 経由 view。arena 全 node の数
        // (detached を含む)。
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

    // NB: id() / has_class() は raikiri-traits::Element の default impl を
    // 使用。default が self.attr(...) 経由で lookup するため、この impl は
    // attr() だけ override すれば id/has_class も追従する (DRY / 契約準拠)。

    fn attr(&self, local: &str) -> Option<&str> {
        self.attr(local)
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
        // Comment / ProcessingInstruction / DocumentFragment arms を追加。
        // cascade / rule-tree walk は Element のみ処理する契約
        // (crates/raikiri-style/src/cascade.rs / ruletree.rs) なので、追加 kind
        // は自動的に non-styling。両 trait family で kind() の projection が
        // 一致することは Two-way invariant の一部。
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

    // NB: id() / has_class() は raikiri_style::StyleElement の default impl を
    // 使用。default が self.attr(...) 経由で lookup するため、この impl は
    // attr() だけ override すれば id/has_class も追従する (DRY / 契約準拠)。

    fn attr(&self, local: &str) -> Option<&str> {
        self.attr(local)
    }
}
