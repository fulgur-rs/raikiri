use raikiri_traits::{NodeId, NodeKind};

/// Read-only structure and attributes of the laid-out document.
///
/// Layout state (taffy boxes, text layouts) is not reachable from here.
/// Every accessor is total: an out-of-range [`NodeId`] yields `None`, an empty
/// iterator or an empty string.
#[derive(Debug, Clone, Copy)]
pub struct DomView<'a> {
    document: &'a raikiri_dom::Document,
}

/// Elements before whose content [`DomView::text_content`] inserts a space,
/// matching the block boundaries fulgur uses for link and heading text.
const BLOCK_BOUNDARY_TAGS: &[&str] = &[
    "br",
    "div",
    "p",
    "li",
    "ul",
    "ol",
    "table",
    "tr",
    "td",
    "th",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "section",
    "article",
    "header",
    "footer",
    "blockquote",
    "pre",
    "hr",
];

impl<'a> DomView<'a> {
    pub(crate) fn new(document: &'a raikiri_dom::Document) -> Self {
        Self { document }
    }

    fn node(&self, node: NodeId) -> Option<&'a raikiri_dom::Node> {
        self.document.get_node(node.0 as usize)
    }

    /// The document node.
    pub fn root(&self) -> NodeId {
        NodeId(self.document.root_index() as u64)
    }

    /// Node kind.
    pub fn kind(&self, node: NodeId) -> Option<NodeKind> {
        self.node(node).map(|n| n.kind())
    }

    /// Parent node.
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.node(node)?;
        self.document
            .parent_of(node.0 as usize)
            .map(|p| NodeId(p as u64))
    }

    /// Children in document order.
    pub fn children(&self, node: NodeId) -> impl Iterator<Item = NodeId> + 'a {
        self.node(node)
            .map(|n| n.children.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|&c| NodeId(c as u64))
    }

    /// Element local name (lower case for HTML elements).
    pub fn local_name(&self, node: NodeId) -> Option<&'a str> {
        self.node(node)?.tag_name()
    }

    /// Element namespace URI. `None` for the HTML namespace fast path.
    pub fn namespace(&self, node: NodeId) -> Option<&'a str> {
        self.node(node)?.namespace_uri()
    }

    /// Attribute value.
    pub fn attr(&self, node: NodeId, name: &str) -> Option<&'a str> {
        self.node(node)?.attribute(name)
    }

    /// A text node's source string.
    pub fn text(&self, node: NodeId) -> Option<&'a str> {
        let n = self.node(node)?;
        (n.kind() == NodeKind::Text)
            .then(|| n.text_content())
            .flatten()
    }

    /// Descendant text in document order. A space is inserted before the
    /// content of a block-boundary element when the text so far does not end
    /// with whitespace. Whitespace is otherwise kept as is.
    pub fn text_content(&self, node: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(node, &mut out);
        out
    }

    fn collect_text(&self, node: NodeId, out: &mut String) {
        let Some(n) = self.node(node) else {
            return;
        };
        if n.kind() == NodeKind::Text {
            if let Some(text) = n.text_content() {
                out.push_str(text);
            }
            return;
        }
        for child in self.children(node) {
            let is_block = self
                .local_name(child)
                .is_some_and(|name| BLOCK_BOUNDARY_TAGS.contains(&name));
            if is_block && !out.is_empty() && !out.ends_with(char::is_whitespace) {
                out.push(' ');
            }
            self.collect_text(child, out);
        }
    }
}
