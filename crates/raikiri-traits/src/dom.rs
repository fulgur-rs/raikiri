//! DOM abstraction trait + identifier newtypes.
//!
//! `Dom` / `Element<'a>` / `Node<'a>` は M1.5 (`dom-model` task) で
//! associated type + method を確定する予定。M1.1 では shell として trait だけ
//! 用意し、raikiri-dom 側の実装検討と co-design する。

use smol_str::SmolStr;

/// String identifier for GCPM fragments / running templates / named strings /
/// target-* references.
///
/// SmolStr newtype で inline 最適化を効かせる。M4 GCPM で本格利用開始。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol(pub SmolStr);

impl Symbol {
    /// Construct from any string type.
    pub fn new(s: impl Into<SmolStr>) -> Self {
        Self(s.into())
    }

    /// Borrow inner str.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// Stable identifier for DOM nodes across a document.
///
/// `RenderWarning.node_id` などで参照される。raikiri-dom は自 arena の
/// node index を u64 に射影して produce。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u64);

impl NodeId {
    /// Construct a node identifier from a raw u64.
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

/// DOM node の種別 (Element / Text / Document root)。
///
/// M1.5 で raikiri-dom node arena の kind field と対応する。将来 (M4)
/// Comment / CDATA / ProcessingInstruction 等が加わる可能性があるため
/// `#[non_exhaustive]`。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// HTML / XML element (tag_name あり)。
    Element,
    /// Character data node。
    Text,
    /// Document root (arena index 0 に配置される仮想 node)。
    Document,
}

/// DOM tree abstraction。raikiri-dom / raikiri-style / raikiri-paint / raikiri
/// (umbrella) が消費する generic navigation interface。
///
/// **Object-safety**: GAT (`type NodeRef<'a>`) を含むため non-object-safe。
/// M1 では generic dispatch (`fn walk<D: Dom>(dom: &D)`) を前提。dyn 化が
/// 必要な場合 (M6 blitz-compat 経由の runtime abstraction 等) は erased
/// wrapper trait を別途用意する。
pub trait Dom {
    /// Node reference (borrowed) type。
    type NodeRef<'a>: Node<'a>
    where
        Self: 'a;
    /// Element reference (borrowed) type。
    type ElementRef<'a>: Element<'a>
    where
        Self: 'a;
    /// Child ID iterator type。
    type ChildIter<'a>: Iterator<Item = NodeId>
    where
        Self: 'a;

    /// Document root node の identifier。実装は通常 arena index 0 の Document
    /// kind node を指す。
    fn root_id(&self) -> NodeId;

    /// `id` に対応する Node reference。範囲外なら `None`。
    fn node(&self, id: NodeId) -> Option<Self::NodeRef<'_>>;

    /// `id` の direct children を走査する iterator。
    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_>;
}

/// Node reference (borrowed lifetime `'a`)。kind ごとの dispatch と共通 API を
/// 提供。
pub trait Node<'a> {
    /// Element downcast 用の Element reference type。
    type Element<'b>: Element<'b>
    where
        Self: 'b;

    /// この node の種別。
    fn kind(&self) -> NodeKind;

    /// kind が Element の場合 Element reference を返す。それ以外 (Text /
    /// Document) は `None`。
    fn as_element(&self) -> Option<Self::Element<'_>>;

    /// kind が Text の場合 character data。それ以外 (Element / Document) は
    /// `None`。
    fn text_content(&self) -> Option<&str>;
}

/// Element reference (borrowed lifetime `'a`)。
///
/// M1.6 以降で attribute / classes / id lookup 等を追加する予定。M1.5 は
/// tag_name のみ確定。
pub trait Element<'a> {
    /// HTML / XML tag name (例: `"p"`, `"div"`)。
    fn tag_name(&self) -> &str;
}
