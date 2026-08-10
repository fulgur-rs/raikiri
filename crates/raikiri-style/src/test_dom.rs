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
    /// Null-namespace attributes other than `style` (bd raikiri-spike-5z86.7 —
    /// needed to exercise `StyleElement::attr()` lookups other than the
    /// `id` / `class` / `style` ones the trait already special-cases).
    /// First-wins on duplicate names, mirroring `ElementRef::attr()`'s
    /// contract in `crates/raikiri-dom/src/dom_impl.rs`.
    pub(crate) attrs: Vec<(String, String)>,
    /// Namespace URI; `None` = HTML default namespace (matches
    /// `StyleElement::namespace_uri`'s "fast path" doc). bd
    /// raikiri-spike-5z86.7 Codex final-review finding 1 — needed to
    /// exercise the HTML-namespace gate on foreign-namespace elements that
    /// happen to share a local name with an HTML element.
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
                tag: String::new(),
                inline_style: None,
                attrs: Vec::new(),
                namespace: None,
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
        self.push_element_with_attrs(parent, tag, inline_style, &[])
    }

    /// [`Self::push_element`] に加えて `style`/`id`/`class` 以外の任意 null-
    /// namespace attribute も設定できる版 (bd raikiri-spike-5z86.7)。既存
    /// call site を壊さないよう `push_element` はこの関数への空 slice 委譲
    /// にした。
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

    /// Non-HTML-namespace element (bd raikiri-spike-5z86.7 Codex
    /// final-review finding 1's regression test — a foreign-namespace
    /// element sharing an HTML local name must not pick up HTML's
    /// presentational hints).
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
}

impl<'a> StyleElement for TestElementRef<'a> {
    fn tag_name(&self) -> &str {
        &self.node.tag
    }
    fn inline_style_source(&self) -> Option<&str> {
        self.node.inline_style.as_deref()
    }
    fn namespace_uri(&self) -> Option<&str> {
        self.node.namespace.as_deref()
    }
    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            self.inline_style_source()
        } else {
            // `.filter(|s| !s.is_empty())`: matches the `StyleElement::attr`
            // trait doc contract ("Empty string is normalised to `None`")
            // and the real `ElementRef::attr()`
            // (`crates/raikiri-dom/src/dom_impl.rs`) it mirrors.
            self.node
                .attrs
                .iter()
                .find(|(k, _)| k == local)
                .map(|(_, v)| v.as_str())
                .filter(|s| !s.is_empty())
        }
    }
}
