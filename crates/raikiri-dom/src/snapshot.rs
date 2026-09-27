//! Identity-preserving logical arena transport; layout caches are rebuilt.
use crate::node::Attr;
use crate::{Document, Node, NodeData, QuirksMode};
use raikiri_traits::StylesheetKind;

/// Owned logical document with arena positions as node identities.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "snapshot-serde",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct LogicalSnapshot {
    /// Root arena index.
    pub root: usize,
    /// All nodes, including detached nodes.
    pub nodes: Vec<LogicalNode>,
    /// 0: standards, 1: limited quirks, 2: quirks.
    pub quirks: u8,
    /// Sources paired with origin (0: UA, 1: user, 2: author).
    pub stylesheets: Vec<(String, u8)>,
}
/// Node contents and ordered children without derived state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "snapshot-serde",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct LogicalNode {
    /// Kind-specific logical data.
    pub data: LogicalData,
    /// Ordered arena indices.
    pub children: Vec<usize>,
}
/// Namespaced attribute in source order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "snapshot-serde",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct LogicalAttr {
    /// Namespace URI.
    pub namespace: Option<String>,
    /// Qualified-name prefix.
    pub prefix: Option<String>,
    /// Local name.
    pub local: String,
    /// Raw value.
    pub value: String,
}
/// Logical node variants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "snapshot-serde",
    derive(serde::Serialize, serde::Deserialize)
)]
pub enum LogicalData {
    /// Document root.
    Document,
    /// Detached fragment root.
    Fragment,
    /// Element and namespace metadata.
    Element {
        /// Local name.
        tag: String,
        /// Namespace URI.
        namespace: Option<String>,
        /// Prefix.
        prefix: Option<String>,
        /// Ordered attributes.
        attributes: Vec<LogicalAttr>,
        /// Raw inline style.
        inline_style: Option<String>,
        /// Template fragment arena index.
        template: Option<usize>,
    },
    /// Text content.
    Text(String),
    /// Comment content.
    Comment(String),
    /// Processing instruction target/data.
    ProcessingInstruction(String, String),
}
/// Invalid logical arena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotError(pub &'static str);
impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for SnapshotError {}

impl LogicalSnapshot {
    /// Validate every component iteratively before rebuilding nodes.
    pub fn validate(&self, max_nodes: usize) -> Result<(), SnapshotError> {
        let fail = |s| Err(SnapshotError(s));
        let n = self.nodes.len();
        if n == 0 || n > max_nodes || self.root >= n {
            return fail("arena bounds");
        }
        if !matches!(
            self.nodes[self.root].data,
            LogicalData::Document | LogicalData::Fragment
        ) {
            return fail("root kind");
        }
        if self.quirks > 2 || self.stylesheets.iter().any(|(_, k)| *k > 2) {
            return fail("document metadata");
        }
        let mut parents = vec![false; n];
        let mut edges = vec![Vec::new(); n];
        for (id, node) in self.nodes.iter().enumerate() {
            if matches!(node.data, LogicalData::Document) && id != self.root {
                return fail("extra document root");
            }
            if matches!(
                node.data,
                LogicalData::Text(_)
                    | LogicalData::Comment(_)
                    | LogicalData::ProcessingInstruction(..)
            ) && !node.children.is_empty()
            {
                return fail("leaf children");
            }
            edges[id].extend_from_slice(&node.children);
            if let LogicalData::Element {
                tag,
                namespace,
                template: Some(t),
                ..
            } = &node.data
            {
                if tag != "template"
                    || namespace
                        .as_deref()
                        .is_some_and(|s| s != "http://www.w3.org/1999/xhtml")
                    || *t >= n
                    || !matches!(self.nodes[*t].data, LogicalData::Fragment)
                {
                    return fail("template link");
                }
                edges[id].push(*t);
            }
            for &child in &edges[id] {
                if child >= n || child == self.root || parents[child] {
                    return fail("edge bounds or repeated parent");
                }
                parents[child] = true;
            }
        }
        let mut colors = vec![0u8; n];
        for start in 0..n {
            if colors[start] != 0 {
                continue;
            }
            let mut stack = vec![(start, false)];
            while let Some((id, exit)) = stack.pop() {
                if exit {
                    colors[id] = 2;
                    continue;
                }
                if colors[id] == 1 {
                    return fail("cycle");
                }
                if colors[id] == 2 {
                    continue;
                }
                colors[id] = 1;
                stack.push((id, true));
                stack.extend(edges[id].iter().rev().map(|&child| (child, false)));
            }
        }
        Ok(())
    }
}
impl Document {
    /// Export the arena without layout, shaping or paint caches.
    pub fn logical_snapshot(&self) -> LogicalSnapshot {
        LogicalSnapshot {
            root: self.root,
            quirks: match self.quirks_mode() {
                QuirksMode::NoQuirks => 0,
                QuirksMode::LimitedQuirks => 1,
                _ => 2,
            },
            stylesheets: self
                .stylesheets()
                .map(|(s, k)| {
                    (
                        s.to_owned(),
                        match k {
                            StylesheetKind::UserAgent => 0,
                            StylesheetKind::User => 1,
                            _ => 2,
                        },
                    )
                })
                .collect(),
            nodes: self
                .nodes
                .iter()
                .map(|n| LogicalNode {
                    children: n.children.clone(),
                    data: match &n.data {
                        NodeData::Document => LogicalData::Document,
                        NodeData::DocumentFragment => LogicalData::Fragment,
                        NodeData::Text(t) => LogicalData::Text(t.text_content.to_string()),
                        NodeData::Comment(t) => LogicalData::Comment(t.to_string()),
                        NodeData::ProcessingInstruction { target, data } => {
                            LogicalData::ProcessingInstruction(target.to_string(), data.to_string())
                        }
                        NodeData::Element(e) => LogicalData::Element {
                            tag: e.tag_name.to_string(),
                            namespace: e.namespace.as_ref().map(ToString::to_string),
                            prefix: e.prefix.as_ref().map(ToString::to_string),
                            inline_style: e.inline_style.as_ref().map(ToString::to_string),
                            template: e.template_contents,
                            attributes: e
                                .attributes
                                .iter()
                                .map(|a| LogicalAttr {
                                    namespace: a.namespace.as_ref().map(ToString::to_string),
                                    prefix: a.prefix.as_ref().map(ToString::to_string),
                                    local: a.local.to_string(),
                                    value: a.value.to_string(),
                                })
                                .collect(),
                        },
                    },
                })
                .collect(),
        }
    }
    /// Rebuild fresh derived state while preserving all arena identities.
    pub fn from_logical_snapshot(
        snapshot: LogicalSnapshot,
        max_nodes: usize,
    ) -> Result<Self, SnapshotError> {
        snapshot.validate(max_nodes)?;
        let mut doc = Self::new();
        doc.root = snapshot.root;
        doc.set_quirks_mode(match snapshot.quirks {
            0 => QuirksMode::NoQuirks,
            1 => QuirksMode::LimitedQuirks,
            _ => QuirksMode::Quirks,
        });
        for (s, k) in snapshot.stylesheets {
            doc.add_stylesheet(
                s,
                match k {
                    0 => StylesheetKind::UserAgent,
                    1 => StylesheetKind::User,
                    _ => StylesheetKind::Author,
                },
            );
        }
        doc.nodes = snapshot
            .nodes
            .into_iter()
            .map(|n| {
                let mut node = match n.data {
                    LogicalData::Document => Node::new_document(),
                    LogicalData::Fragment => Node::new_document_fragment(),
                    LogicalData::Text(s) => Node::new_text(s.into()),
                    LogicalData::Comment(s) => Node::new_comment(s.into()),
                    LogicalData::ProcessingInstruction(t, s) => {
                        Node::new_processing_instruction(t.into(), s.into())
                    }
                    LogicalData::Element {
                        tag,
                        namespace,
                        prefix,
                        attributes,
                        inline_style,
                        template,
                    } => {
                        let mut node = Node::new_element(
                            tag.into(),
                            Default::default(),
                            inline_style.map(Into::into),
                        );
                        let e = node.data.as_element_mut().expect("element constructor");
                        e.namespace = namespace.map(Into::into);
                        e.prefix = prefix.map(Into::into);
                        e.template_contents = template;
                        e.attributes = attributes
                            .into_iter()
                            .map(|a| Attr {
                                namespace: a.namespace.map(Into::into),
                                prefix: a.prefix.map(Into::into),
                                local: a.local.into(),
                                value: a.value.into(),
                            })
                            .collect();
                        node
                    }
                };
                node.children = n.children;
                node
            })
            .collect();
        doc.flags_dirty = true;
        doc.mark_in_document_flags();
        Ok(doc)
    }
}
#[cfg(test)]
mod tests;
