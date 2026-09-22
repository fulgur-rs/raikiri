//! crate-internal test helper。`StyleDom` / `StyleNode` / `StyleElement` を
//! 最小実装した builder-style mock。raikiri-dom crate への test 依存を回避し、
//! raikiri-style を standalone に unit test できるようにする。
//!

use crate::style_dom::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};

pub(crate) struct TestDoc {
    pub(crate) nodes: Vec<TestNode>,
    /// Document-mode context. Defaults to
    /// `NoQuirks`, matching [`StyleDom::quirks_mode`]'s own default —
    /// Callers assign this field directly when constructing a fixture.
    pub(crate) quirks_mode: StyleQuirksMode,
}

pub(crate) struct TestNode {
    pub(crate) kind: StyleNodeKind,
    pub(crate) in_document: bool,
    pub(crate) tag: String,
    pub(crate) inline_style: Option<String>,
    /// Null-namespace attributes other than `style`. First-wins on duplicate
    /// names, matching the element lookup contract.
    pub(crate) attrs: Vec<(String, String)>,
    /// Namespace URI; `None` represents the HTML default namespace.
    pub(crate) namespace: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) children: Vec<usize>,
}

impl TestDoc {
    pub(crate) fn new() -> Self {
        // index 0 = Document root。
        Self {
            nodes: vec![TestNode {
                kind: StyleNodeKind::Document,
                in_document: true,
                tag: String::new(),
                inline_style: None,
                attrs: Vec::new(),
                namespace: None,
                text: None,
                children: Vec::new(),
            }],
            quirks_mode: StyleQuirksMode::NoQuirks,
        }
    }

    /// Element を追加。parent は既存 index。
    pub(crate) fn push_element(
        &mut self,
        parent: usize,
        tag: &str,
        inline_style: Option<&str>,
    ) -> usize {
        self.push_element_with_attrs(parent, tag, inline_style, &[])
    }

    /// Push an element with additional null-namespace attributes.
    pub(crate) fn push_element_with_attrs(
        &mut self,
        parent: usize,
        tag: &str,
        inline_style: Option<&str>,
        attrs: &[(&str, &str)],
    ) -> usize {
        let id = self.nodes.len();
        self.nodes.push(TestNode {
            kind: StyleNodeKind::Element,
            in_document: true,
            tag: tag.into(),
            inline_style: inline_style.map(|s| s.to_string()),
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            namespace: None,
            text: None,
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }

    /// Push an element in a non-HTML namespace.
    pub(crate) fn push_element_with_namespace(
        &mut self,
        parent: usize,
        tag: &str,
        namespace: &str,
        attrs: &[(&str, &str)],
    ) -> usize {
        let id = self.nodes.len();
        self.nodes.push(TestNode {
            kind: StyleNodeKind::Element,
            in_document: true,
            tag: tag.into(),
            inline_style: None,
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            namespace: Some(namespace.to_string()),
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
            in_document: true,
            tag: String::new(),
            inline_style: None,
            attrs: Vec::new(),
            namespace: None,
            text: Some(text.into()),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }

    /// Comment node (needed to exercise
    /// `:empty`'s "comments... must not affect whether an element is
    /// considered empty" clause, CSS Selectors L3 §6.6.4 verbatim, see
    /// `cascade.rs`'s `matches_empty` doc).
    pub(crate) fn push_comment(&mut self, parent: usize, text: &str) -> usize {
        let id = self.nodes.len();
        self.nodes.push(TestNode {
            kind: StyleNodeKind::Comment,
            in_document: true,
            tag: String::new(),
            inline_style: None,
            attrs: Vec::new(),
            namespace: None,
            text: Some(text.into()),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }

    /// Set a null-namespace attribute on an existing element. Attribute names
    /// are expected to be lower-case.
    pub(crate) fn set_attr(&mut self, id: usize, name: &str, value: &str) {
        self.nodes[id]
            .attrs
            .push((name.to_string(), value.to_string()));
    }

    /// Override an existing element's namespace URI.
    pub(crate) fn set_namespace(&mut self, id: usize, namespace_uri: &str) {
        self.nodes[id].namespace = Some(namespace_uri.to_string());
    }

    /// Override flat-tree membership for a node. This lets style tests model
    /// detached or inert nodes while keeping raw arena child order intact.
    pub(crate) fn set_in_document(&mut self, id: usize, in_document: bool) {
        self.nodes[id].in_document = in_document;
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
        // Contract-align with `node()`: invalid StyleNodeId → empty iter.
        let slice = self
            .nodes
            .get(idx)
            .map(|n| n.children.as_slice())
            .unwrap_or(&[]);
        TestChildIter(slice.iter())
    }

    fn node_count(&self) -> usize {
        // cascade が out.resize() の pre-allocation で消費する。
        self.nodes.len()
    }

    fn quirks_mode(&self) -> StyleQuirksMode {
        self.quirks_mode
    }
}

impl<'a> StyleNode for TestNodeRef<'a> {
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

    fn is_in_document(&self) -> bool {
        self.doc.nodes[self.id].in_document
    }
}

impl<'a> StyleElement for TestElementRef<'a> {
    fn tag_name(&self) -> &str {
        &self.node.tag
    }
    /// `.filter(|s| !s.is_empty())`: mirrors the real
    /// `ElementRef::inline_style_source()` (`crates/raikiri-dom/src/dom_impl.rs`),
    /// which has its own "empty `style=""` is `None`" contract (see
    /// `StyleElement::inline_style_source`'s trait doc). This field is
    /// unaffected by the presence/value split `attr()` applies to other
    /// attributes below — `style` is carved out as an exception to that
    /// split on both the real DOM and this mock.
    fn inline_style_source(&self) -> Option<&str> {
        self.node.inline_style.as_deref().filter(|s| !s.is_empty())
    }
    fn namespace_uri(&self) -> Option<&str> {
        self.node.namespace.as_deref()
    }
    /// Attribute lookup for the test DOM. Empty values are treated as absent
    /// by this lightweight mock; production DOM implementations must follow
    /// [`StyleElement::attr`]'s presence contract.
    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            return self.inline_style_source();
        }
        self.node
            .attrs
            .iter()
            .find(|(k, _)| k == local)
            .map(|(_, v)| v.as_str())
            .filter(|s| !s.is_empty())
    }
}
