//! Document arena — Vec-backed node arena that implements taffy layout traits
//! (in `taffy_impl.rs`) and raikiri-traits::Dom (in `dom_impl.rs`).

use smol_str::SmolStr;
use taffy::Style;

use crate::node::Node;

/// DOM Document (root + Vec-backed node arena)。
///
/// `nodes` は arena indices を key とする flat storage。index 0 は Document
/// kind の virtual root。HTML の `<html>` element は M1.3 html-parse-basic が
/// index 1 以降に append する想定 (root = 0 の子として)。
pub struct Document {
    pub(crate) nodes: Vec<Node>,
    /// arena index of the Document root (always 0 の予定、明示的に保持して
    /// 将来 detach root 等の変則 case に備える)。
    pub(crate) root: usize,
}

impl Document {
    /// 新しい Document を構築する。arena index 0 に Document kind node を配置。
    pub fn new() -> Self {
        let mut nodes = Vec::with_capacity(16);
        nodes.push(Node::new_document());
        Self { nodes, root: 0 }
    }

    /// Element node を arena に追加する。`parent` が `Some(idx)` の場合
    /// その node の children に append される。`None` の場合 detached (どこにも
    /// 属さない fragment、後で attach する用途)。
    ///
    /// Returns: 追加された node の arena index。
    pub fn append_element(
        &mut self,
        parent: Option<usize>,
        tag: impl Into<SmolStr>,
        style: Style,
    ) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_element(tag.into(), style));
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
        }
        id
    }

    /// Text node を arena に追加する。`parent` の子として append される
    /// (parent は必須、text は必ず attach される)。
    ///
    /// Returns: 追加された node の arena index。
    pub fn append_text(&mut self, parent: usize, text: impl Into<SmolStr>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node::new_text(text.into()));
        self.nodes[parent].children.push(id);
        id
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}
