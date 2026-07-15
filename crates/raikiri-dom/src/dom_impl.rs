//! `raikiri_traits::Dom / Node / Element` implementations on Document.
//!
//! `NodeRef<'a>` と `ElementRef<'a>` は Document の内部 arena を borrow する
//! 軽量 wrapper。GAT 経由で trait method の返り値型を安定させる。

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
        ChildIter(self.nodes[idx].children.iter())
    }
}

impl<'a> raikiri_traits::Node<'a> for NodeRef<'a> {
    type Element<'b>
        = ElementRef<'b>
    where
        Self: 'b;

    fn kind(&self) -> NodeKind {
        self.doc.nodes[self.id].kind
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
        let node = &self.doc.nodes[self.id];
        matches!(node.kind, NodeKind::Element).then(|| ElementRef { node })
    }

    fn text_content(&self) -> Option<&str> {
        self.doc.nodes[self.id].text_content.as_deref()
    }
}

impl<'a> raikiri_traits::Element<'a> for ElementRef<'a> {
    fn tag_name(&self) -> &str {
        self.node.tag_name.as_deref().unwrap_or("")
    }

    fn inline_style_source(&self) -> Option<&str> {
        // Trait contract: 空文字列 `style=""` は `None` を返す。
        // 内部 field が `Some(SmolStr::new(""))` の場合も boundary で捨てる。
        self.node.inline_style.as_deref().filter(|s| !s.is_empty())
    }

    fn namespace_uri(&self) -> Option<&str> {
        // HTML default namespace は Node.namespace = None として格納しているので
        // そのまま返せばよい (fast path 済)。
        self.node.namespace.as_deref()
    }

    fn id(&self) -> Option<&str> {
        // `id` は attr 一般 lookup と同じ空文字列正規化契約。
        self.node
            .attributes
            .iter()
            .find(|a| a.local == "id")
            .map(|a| a.value.as_str())
            .filter(|s| !s.is_empty())
    }

    fn has_class(&self, class: &str) -> bool {
        if class.is_empty() {
            return false;
        }
        let Some(class_attr) = self.node.attributes.iter().find(|a| a.local == "class") else {
            return false;
        };
        // HTML spec: ASCII whitespace で split (space, tab, LF, CR, FF)。
        // class 属性 value を token 単位で比較 (大文字小文字 sensitive: HTML
        // classList spec は case-sensitive)。
        class_attr
            .value
            .split([' ', '\t', '\n', '\r', '\x0C'])
            .any(|token| token == class)
    }

    fn attr(&self, local: &str) -> Option<&str> {
        // `style` は Node.inline_style に分離済のため attributes からは
        // 探しに行かず inline_style を返す (trait doc の一致性契約)。
        if local == "style" {
            return self.inline_style_source();
        }
        self.node
            .attributes
            .iter()
            .find(|a| a.local == local)
            .map(|a| a.value.as_str())
            .filter(|s| !s.is_empty())
    }
}
