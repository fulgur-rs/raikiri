//! crate-internal test helper。`StyleDom` / `StyleNode` / `StyleElement` を
//! 最小実装した builder-style mock。raikiri-dom crate への test 依存を回避し、
//! raikiri-style を standalone に unit test できるようにする。
//!
//! **意図的に compat blanket に依存しない**: この impl は Phase A の "style-owned
//! trait 面が独り立ちできる" ことを示す証明 — `style_dom` module の blanket
//! (Phase B で除去予定) を外しても TestDoc はそのまま cascade / ruletree に
//! 食わせられる。

use crate::style_dom::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};

pub(crate) struct TestDoc {
    pub(crate) nodes: Vec<TestNode>,
    /// Document-mode context. Defaults to
    /// `NoQuirks`, matching [`StyleDom::quirks_mode`]'s own default —
    /// callers assign this field directly for quirks-mode regression tests
    /// (no setter: no arena-indexing work to wrap, unlike
    /// [`Self::set_attr`] / [`Self::set_namespace`]).
    pub(crate) quirks_mode: StyleQuirksMode,
}

pub(crate) struct TestNode {
    pub(crate) kind: StyleNodeKind,
    pub(crate) tag: String,
    pub(crate) inline_style: Option<String>,
    /// Null-namespace attributes other than `style` —
    /// needed to exercise `StyleElement::attr()` lookups other than the
    /// `id` / `class` / `style` ones the trait already special-cases, and
    /// for class/id/attribute selector matching tests, which added the
    /// post-hoc [`TestDoc::set_attr`] setter as a second way to reach it
    /// (both sets of tests read this one field). First-wins on duplicate names,
    /// mirroring `ElementRef::attr()`'s contract in
    /// `crates/raikiri-dom/src/dom_impl.rs`.
    pub(crate) attrs: Vec<(String, String)>,
    /// Namespace URI; `None` = HTML default namespace (matches
    /// `StyleElement::namespace_uri`'s "fast path" doc). Needed to
    /// exercise the HTML-namespace gate on foreign-namespace elements that
    /// happen to share a local name with an HTML element, and to exercise
    /// `resolve_case_sensitivity`'s non-HTML-namespace branch (via the
    /// post-hoc [`TestDoc::set_namespace`] setter).
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

    /// [`Self::push_element`] に加えて `style`/`id`/`class` 以外の任意 null-
    /// namespace attribute も設定できる版。既存
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

    /// Non-HTML-namespace element (regression test — a foreign-namespace
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

    /// Comment node (needed to exercise
    /// `:empty`'s "comments... must not affect whether an element is
    /// considered empty" clause, CSS Selectors L3 §6.6.4 verbatim, see
    /// `cascade.rs`'s `matches_empty` doc).
    pub(crate) fn push_comment(&mut self, parent: usize, text: &str) -> usize {
        let id = self.nodes.len();
        self.nodes.push(TestNode {
            kind: StyleNodeKind::Comment,
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

    /// Set a null-namespace attribute (e.g. `class`, `id`, `data-foo`) on an
    /// already-pushed element — a post-hoc
    /// alternative to [`Self::push_element_with_attrs`]'s constructor-time
    /// form, for call sites that only decide which attrs to add after
    /// already having the element's id. `name` should already be
    /// lower-case — this mock does not itself lower-case, mirroring how a
    /// real HTML parser hands `raikiri-style` already-lowercased attribute
    /// names.
    pub(crate) fn set_attr(&mut self, id: usize, name: &str, value: &str) {
        self.nodes[id]
            .attrs
            .push((name.to_string(), value.to_string()));
    }

    /// Override an already-pushed element's namespace URI — a post-hoc
    /// alternative to
    /// [`Self::push_element_with_namespace`]'s constructor-time form.
    /// Exercises `resolve_case_sensitivity`'s non-HTML-namespace branch,
    /// which the default `None` (HTML) never reaches.
    pub(crate) fn set_namespace(&mut self, id: usize, namespace_uri: &str) {
        self.nodes[id].namespace = Some(namespace_uri.to_string());
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
    /// Overrides the default (`"style"`-only) `attr()` to also serve attrs
    /// set via [`TestDoc::set_attr`] / [`TestDoc::push_element_with_attrs`]
    /// — needed so the inherited default `id()` / `has_class()` impls
    /// (which delegate to `attr()`) exercise the same code path a real
    /// `StyleElement` impl would.
    ///
    /// Deliberately keeps the older "empty value is normalised to `None`"
    /// behavior as a simplification local to this mock, rather than
    /// matching the real DOM's current presence/value-independent contract
    /// (`StyleElement::attr`'s trait doc, and
    /// `raikiri-dom::dom_impl::ElementRef::attr`, the impl that actually
    /// honors it). This is a known, accepted divergence between `TestDoc`
    /// and the real DOM: selector-matching behavior that depends on
    /// empty-value attribute presence (`[foo]` / `[foo=""]` against
    /// `foo=""`) must be exercised against the real DOM to be meaningful —
    /// `cascade::tests::attribute_exists_selector_does_not_match_empty_value_attr`
    /// and its `[foo=""]` counterpart pin this mock's own (narrower)
    /// behavior specifically, not the real DOM's.
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
