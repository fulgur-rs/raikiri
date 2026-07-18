//! crate-internal test helper。`StyleDom` / `StyleNode` / `StyleElement` を
//! 最小実装した builder-style mock。raikiri-dom crate への test 依存を回避し、
//! raikiri-style を standalone に unit test できるようにする。
//!
//! **意図的に compat blanket に依存しない**: この impl は Phase A の "style-owned
//! trait 面が独り立ちできる" ことを示す証明 — `style_dom` module の blanket
//! (Phase B で除去予定) を外しても TestDoc はそのまま cascade / ruletree に
//! 食わせられる。

use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};

pub(crate) struct TestDoc {
    pub(crate) nodes: Vec<TestNode>,
}

pub(crate) struct TestNode {
    pub(crate) kind: StyleNodeKind,
    pub(crate) tag: String,
    pub(crate) inline_style: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) children: Vec<usize>,
}

impl TestDoc {
    pub(crate) fn new() -> Self {
        // index 0 = Document root。
        Self {
            nodes: vec![TestNode {
                kind: StyleNodeKind::Document,
                tag: String::new(),
                inline_style: None,
                text: None,
                children: Vec::new(),
            }],
        }
    }

    /// Element を追加。parent は既存 index。
    pub(crate) fn push_element(
        &mut self,
        parent: usize,
        tag: &str,
        inline_style: Option<&str>,
    ) -> usize {
        let id = self.nodes.len();
        self.nodes.push(TestNode {
            kind: StyleNodeKind::Element,
            tag: tag.into(),
            inline_style: inline_style.map(|s| s.to_string()),
            text: None,
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }

    pub(crate) fn push_text(&mut self, parent: usize, text: &str) -> usize {
        let id = self.nodes.len();
        self.nodes.push(TestNode {
            kind: StyleNodeKind::Text,
            tag: String::new(),
            inline_style: None,
            text: Some(text.into()),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }
}

pub(crate) struct TestNodeRef<'a> {
    doc: &'a TestDoc,
    id: usize,
}

pub(crate) struct TestElementRef<'a> {
    node: &'a TestNode,
}

pub(crate) struct TestChildIter<'a>(std::slice::Iter<'a, usize>);

impl Iterator for TestChildIter<'_> {
    type Item = StyleNodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(|i| StyleNodeId::new(i as u64))
    }
}

impl StyleDom for TestDoc {
    type NodeRef<'a> = TestNodeRef<'a>;
    type ElementRef<'a> = TestElementRef<'a>;
    type ChildIter<'a> = TestChildIter<'a>;

    fn root_id(&self) -> StyleNodeId {
        StyleNodeId::new(0)
    }

    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>> {
        let idx = id.0 as usize;
        (idx < self.nodes.len()).then_some(TestNodeRef { doc: self, id: idx })
    }

    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        let idx = id.0 as usize;
        // Contract-align with `node()`: invalid StyleNodeId → empty iter (raikiri-spike-ajy).
        let slice = self
            .nodes
            .get(idx)
            .map(|n| n.children.as_slice())
            .unwrap_or(&[]);
        TestChildIter(slice.iter())
    }

    fn node_count(&self) -> usize {
        // raikiri-spike-37c: cascade が out.resize() の pre-allocation で消費する。
        self.nodes.len()
    }
}

impl<'a> StyleNode<'a> for TestNodeRef<'a> {
    type Element<'b>
        = TestElementRef<'b>
    where
        Self: 'b;
    fn kind(&self) -> StyleNodeKind {
        self.doc.nodes[self.id].kind
    }
    fn as_element(&self) -> Option<Self::Element<'_>> {
        matches!(self.kind(), StyleNodeKind::Element).then(|| TestElementRef {
            node: &self.doc.nodes[self.id],
        })
    }
    fn text_content(&self) -> Option<&str> {
        self.doc.nodes[self.id].text.as_deref()
    }
}

impl<'a> StyleElement<'a> for TestElementRef<'a> {
    fn tag_name(&self) -> &str {
        &self.node.tag
    }
    fn inline_style_source(&self) -> Option<&str> {
        self.node.inline_style.as_deref()
    }
}
