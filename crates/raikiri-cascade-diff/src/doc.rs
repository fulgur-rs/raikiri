//! An arena DOM for generated cases, implementing the `StyleDom` traits.

use raikiri_style::{
    StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind, StyleQuirksMode,
};

/// SVG namespace URI, used for generated `<svg>` subtrees.
pub(crate) const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// One generated node.
pub(crate) struct GenNode {
    pub(crate) kind: StyleNodeKind,
    pub(crate) tag: String,
    pub(crate) namespace: Option<&'static str>,
    /// Null-namespace attributes other than `style`, first one wins.
    pub(crate) attrs: Vec<(String, String)>,
    pub(crate) style: Option<String>,
    pub(crate) animation: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) children: Vec<usize>,
    pub(crate) parent: Option<usize>,
    pub(crate) in_document: bool,
}

impl GenNode {
    fn new(kind: StyleNodeKind) -> Self {
        Self {
            kind,
            tag: String::new(),
            namespace: None,
            attrs: Vec::new(),
            style: None,
            animation: None,
            text: None,
            children: Vec::new(),
            parent: None,
            in_document: true,
        }
    }

    /// A new element node with the given tag name.
    pub(crate) fn element(tag: &str) -> Self {
        Self {
            tag: tag.to_owned(),
            ..Self::new(StyleNodeKind::Element)
        }
    }

    /// A new text node.
    pub(crate) fn text(text: &str) -> Self {
        Self {
            text: Some(text.to_owned()),
            ..Self::new(StyleNodeKind::Text)
        }
    }

    /// A new comment node.
    pub(crate) fn comment(text: &str) -> Self {
        Self {
            text: Some(text.to_owned()),
            ..Self::new(StyleNodeKind::Comment)
        }
    }
}

/// A generated document. Node 0 is the Document node.
pub(crate) struct GenDoc {
    pub(crate) nodes: Vec<GenNode>,
    pub(crate) quirks: StyleQuirksMode,
}

impl GenDoc {
    /// An empty document in the given quirks mode.
    pub(crate) fn new(quirks: StyleQuirksMode) -> Self {
        Self {
            nodes: vec![GenNode::new(StyleNodeKind::Document)],
            quirks,
        }
    }

    /// Appends `node` as the last child of `parent` and returns its index.
    pub(crate) fn append(&mut self, parent: usize, mut node: GenNode) -> usize {
        let id = self.nodes.len();
        node.parent = Some(parent);
        node.in_document &= self.nodes[parent].in_document;
        self.nodes.push(node);
        self.nodes[parent].children.push(id);
        id
    }
}

/// Borrowed handle to a node of a [`GenDoc`].
pub(crate) struct GenNodeRef<'a> {
    node: &'a GenNode,
}

/// Borrowed handle to an element node of a [`GenDoc`].
pub(crate) struct GenElementRef<'a> {
    node: &'a GenNode,
}

/// Child-id iterator over a node's `children` vector.
pub(crate) struct GenChildIter<'a>(std::slice::Iter<'a, usize>);

impl Iterator for GenChildIter<'_> {
    type Item = StyleNodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|&i| StyleNodeId::new(i as u64))
    }
}

impl StyleDom for GenDoc {
    type NodeRef<'a> = GenNodeRef<'a>;
    type ChildIter<'a> = GenChildIter<'a>;

    fn root_id(&self) -> StyleNodeId {
        StyleNodeId::new(0)
    }

    fn node(&self, id: StyleNodeId) -> Option<Self::NodeRef<'_>> {
        let index = usize::try_from(id.0).ok()?;
        self.nodes.get(index).map(|node| GenNodeRef { node })
    }

    fn child_ids(&self, id: StyleNodeId) -> Self::ChildIter<'_> {
        let children = usize::try_from(id.0)
            .ok()
            .and_then(|index| self.nodes.get(index))
            .map_or(&[][..], |node| node.children.as_slice());
        GenChildIter(children.iter())
    }

    fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn quirks_mode(&self) -> StyleQuirksMode {
        self.quirks
    }

    fn parent_id(&self, child: StyleNodeId) -> Option<StyleNodeId> {
        let index = usize::try_from(child.0).ok()?;
        self.nodes
            .get(index)?
            .parent
            .map(|parent| StyleNodeId::new(parent as u64))
    }
}

impl StyleNode for GenNodeRef<'_> {
    type Element<'b>
        = GenElementRef<'b>
    where
        Self: 'b;

    fn kind(&self) -> StyleNodeKind {
        self.node.kind
    }

    fn as_element(&self) -> Option<Self::Element<'_>> {
        (self.node.kind == StyleNodeKind::Element).then_some(GenElementRef { node: self.node })
    }

    fn text_content(&self) -> Option<&str> {
        self.node.text.as_deref()
    }

    fn is_in_document(&self) -> bool {
        self.node.in_document
    }
}

impl StyleElement for GenElementRef<'_> {
    fn tag_name(&self) -> &str {
        &self.node.tag
    }

    fn inline_style_source(&self) -> Option<&str> {
        self.node.style.as_deref()
    }

    fn animation_style_source(&self) -> Option<&str> {
        self.node.animation.as_deref()
    }

    fn namespace_uri(&self) -> Option<&str> {
        self.node.namespace
    }

    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            return self.inline_style_source();
        }
        self.node
            .attrs
            .iter()
            .find(|(name, _)| name == local)
            .map(|(_, value)| value.as_str())
    }
}

#[cfg(test)]
mod tests;
