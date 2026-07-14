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

/// DOM tree abstraction consumed by raikiri-dom / raikiri-style / raikiri-paint.
///
/// M1.1 では shell (method 未定義)。M1.5 `dom-model` で associated type と
/// query method を確定する予定。設計仕様書 §4 参照。
pub trait Dom {
    // M1.5 で populate:
    //   type ElementRef<'a>: Element<'a>;
    //   fn document_element(&self) -> Self::ElementRef<'_>;
    //   fn node(&self, id: NodeId) -> Option<Self::NodeRef<'_>>;
    //   ...
}

/// Element reference abstraction (M1.5 で拡充)。
pub trait Element<'a> {
    // M1.5 で populate:
    //   fn tag_name(&self) -> &str;
    //   fn attribute(&self, name: &str) -> Option<&str>;
    //   ...
}

/// Node reference abstraction (M1.5 で拡充)。
pub trait Node<'a> {
    // M1.5 で populate:
    //   fn node_type(&self) -> NodeKind;
    //   ...
}
