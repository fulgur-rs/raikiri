//! crate-internal test helper。raikiri-traits::{Dom, Node, Element} を最小
//! 実装した builder-style mock。raikiri-dom crate への test 依存を回避し、
//! raikiri-style を standalone に unit test できるようにする。

use raikiri_traits::{Dom, Element, Node, NodeId, NodeKind};

pub(crate) struct TestDoc {
    pub(crate) nodes: Vec<TestNode>,
}

pub(crate) struct TestNode {
    pub(crate) kind: NodeKind,
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
                kind: NodeKind::Document,
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
            kind: NodeKind::Element,
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
            kind: NodeKind::Text,
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
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied().map(|i| NodeId::new(i as u64))
    }
}

impl Dom for TestDoc {
    type NodeRef<'a> = TestNodeRef<'a>;
    type ElementRef<'a> = TestElementRef<'a>;
    type ChildIter<'a> = TestChildIter<'a>;

    fn root_id(&self) -> NodeId {
        NodeId::new(0)
    }

    fn node(&self, id: NodeId) -> Option<Self::NodeRef<'_>> {
        let idx = id.0 as usize;
        (idx < self.nodes.len()).then_some(TestNodeRef { doc: self, id: idx })
    }

    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_> {
        let idx = id.0 as usize;
        // Contract-align with `node()`: invalid NodeId → empty iter (raikiri-spike-ajy).
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

impl<'a> Node<'a> for TestNodeRef<'a> {
    type Element<'b>
        = TestElementRef<'b>
    where
        Self: 'b;
    fn kind(&self) -> NodeKind {
        self.doc.nodes[self.id].kind
    }
    fn as_element(&self) -> Option<Self::Element<'_>> {
        matches!(self.kind(), NodeKind::Element).then(|| TestElementRef {
            node: &self.doc.nodes[self.id],
        })
    }
    fn text_content(&self) -> Option<&str> {
        self.doc.nodes[self.id].text.as_deref()
    }
}

impl<'a> Element<'a> for TestElementRef<'a> {
    fn tag_name(&self) -> &str {
        &self.node.tag
    }
    fn inline_style_source(&self) -> Option<&str> {
        self.node.inline_style.as_deref()
    }
}
