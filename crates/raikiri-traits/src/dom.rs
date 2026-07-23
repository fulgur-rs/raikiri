//! DOM abstraction trait + identifier newtypes.
//!
//! `Dom` / `Element` / `Node` は M1.5 (`dom-model` task) で
//! associated type + method を確定する予定。M1.1 では shell として trait だけ
//! 用意し、raikiri-dom 側の実装検討と co-design する。

use smol_str::SmolStr;

/// String identifier for GCPM fragments / running templates / named strings /
/// target-* references.
///
/// SmolStr newtype で inline 最適化を効かせる。M4 GCPM で本格利用開始。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// DOM node の種別 (Element / Text / Document root / Comment /
/// ProcessingInstruction / DocumentFragment)。
///
/// M1.5 で raikiri-dom node arena の kind field と対応する。
/// raikiri-spike-84y (Sprint 20) で `Comment` / `ProcessingInstruction` /
/// `DocumentFragment` を追加 (WHATWG DOM §4 で列挙された NodeType のうち
/// paged-media rendering に関係する 3 種)。`#[non_exhaustive]` により変更は
/// non-breaking (existing callers は wildcard arm または `matches!(_, Element)`
/// 形式で match するため影響なし)。
///
/// Two-way invariant ([`Node::kind`] / [`Node::as_element`]):
/// `kind() == NodeKind::Element` iff `as_element().is_some()`。追加された
/// `Comment` / `ProcessingInstruction` / `DocumentFragment` はすべて
/// `as_element() == None`。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// HTML / XML element (tag_name あり)。
    Element,
    /// Character data node。
    Text,
    /// Document root (arena index 0 に配置される仮想 node)。
    Document,
    /// HTML / XML comment node (`<!-- ... -->`)。character data を保持する
    /// が Element ではない (`as_element() == None`)。cascade / paint / layout
    /// traversal は typically `is_in_document()` gate で skip されるが、DOM
    /// mutation API の対象としては存在する。raikiri-spike-84y で追加。
    Comment,
    /// Processing instruction node (`<?target data?>`、HTML では実質発生
    /// しないが XML / XHTML では有効)。target + data を保持する。
    /// raikiri-spike-84y で追加。
    ProcessingInstruction,
    /// Document fragment root (`<template>` の contents fragment root や
    /// createDocumentFragment 相当の detached subtree の virtual root)。
    /// arena 内に detached 状態で存在し、Document root からは reachable
    /// でない (mark_in_document_flags 後 `is_in_document() == false`)。
    /// raikiri-spike-84y で `<template>` fragment root の shape 修正のため
    /// 追加 (旧: `"#document-fragment"` pseudo-tag な Element)。
    DocumentFragment,
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
    type NodeRef<'a>: Node
    where
        Self: 'a;
    /// Element reference (borrowed) type。
    type ElementRef<'a>: Element
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

    /// `id` の direct children を走査する iterator。範囲外 (invalid NodeId)
    /// なら empty iterator を返す ([`node`](Self::node) の `None` と対称)。
    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_>;

    /// Arena 内の総 node 数 (Document root および detached / unreachable node
    /// を含む) (raikiri-spike-37c, roborev job 293 M1 finding 対応)。
    ///
    /// cascade などの traversal が `Vec<T>` を pre-allocate する用途で使う。
    /// **契約**: すべての `NodeId(0..node_count as u64)` が [`node`](Self::node)
    /// で `Some` を返すこと。逆に `id.0 >= node_count as u64` なら `None` を
    /// 返す。
    ///
    /// Default impl は `0` を返す。既存 caller が壊れないための safe fallback
    /// で、`Dom` を実装する新しい type は override すべき。既存の raikiri-dom
    /// および raikiri-style の TestDoc impl は override 済み。
    fn node_count(&self) -> usize {
        0
    }
}

/// DOM node abstraction。kind ごとの dispatch と共通 API を提供。
///
/// **Lifetime elision** (raikiri-spike-yxq): 過去は `Node<'a>` の form を持って
/// いたが、method signature で `'a` を使用しないため M1.5 whole-branch review で
/// drop。borrowed Node value 自体の lifetime は `Dom::NodeRef<'a>` の `'a` で
/// 表現されるため trait param 側は不要。GAT `Element<'b>` は borrowed element
/// reference の型として残る。
pub trait Node {
    /// Element downcast 用の Element reference type。
    type Element<'b>: Element
    where
        Self: 'b;

    /// この node の種別。
    fn kind(&self) -> NodeKind;

    /// kind が Element の場合 Element reference を返す。それ以外 (Text /
    /// Document / Comment / ProcessingInstruction / DocumentFragment) は
    /// `None` — Two-way invariant で pinned (`kind() == NodeKind::Element` iff
    /// `as_element().is_some()`)。raikiri-spike-84y で新 3 variant 追加。
    fn as_element(&self) -> Option<Self::Element<'_>>;

    /// kind が Text の場合 character data。それ以外 (Element / Document /
    /// Comment / ProcessingInstruction / DocumentFragment) は `None`。
    /// Comment / PI が character data 相当を持つ場合でも本 method は Text
    /// variant のみを返す (kind 分岐で明示区別、raikiri-spike-84y)。
    fn text_content(&self) -> Option<&str>;

    /// この Node が flat tree に含まれるかを返す。`<template>` element の子孫
    /// は `false`、Document root から flat-tree-parent 経由で到達可能な node
    /// は `true` (raikiri-spike-37c)。
    ///
    /// Traversal 側 (cascade / paint / stylesheet extract) はこの predicate
    /// で inert subtree を統一的に skip する。個別の tag_name 判定
    /// (`== "template"` 等) を traversal に散らすのは禁止 — 概念が implicit
    /// になり shadow DOM 追加時に漏れる。
    ///
    /// # Default impl
    ///
    /// 常に `true` を返す。概念未対応の Node impl (test 用 stub 等) が silent
    /// drop されないための safe fallback (blitz `stylo.rs` `TElement::is_in_document
    /// -> true` と同じ姿勢)。raikiri-dom `NodeRef` は override して実 bit を
    /// 返す。
    ///
    /// ```
    /// use raikiri_traits::Node;
    /// # struct DummyNode;
    /// # struct DummyElem;
    /// # impl raikiri_traits::Element for DummyElem { fn tag_name(&self) -> &str { "" } }
    /// # impl Node for DummyNode {
    /// #   type Element<'b> = DummyElem where Self: 'b;
    /// #   fn kind(&self) -> raikiri_traits::NodeKind { raikiri_traits::NodeKind::Document }
    /// #   fn as_element(&self) -> Option<Self::Element<'_>> { None }
    /// #   fn text_content(&self) -> Option<&str> { None }
    /// # }
    /// // Default impl は常に true — 概念未対応の実装は overriding 不要。
    /// let n = DummyNode;
    /// assert!(n.is_in_document());
    /// ```
    fn is_in_document(&self) -> bool {
        true
    }
}

/// DOM element abstraction。
///
/// `tag_name` は M1.5 で確定。`inline_style_source` / `namespace_uri` /
/// `id` / `has_class` / `attr` は raikiri-spike-blg で追加。attribute lookup は
/// null-namespace attr のみ (namespaced attr = xlink:href 等は M2+ に defer)。
///
/// **Lifetime elision** (raikiri-spike-yxq): 過去は `Element<'a>` の form を
/// 持っていたが、method signature で `'a` を使用しないため M1.5 whole-branch
/// review で drop。borrowed Element value 自体の lifetime は `Dom::ElementRef<'a>`
/// の `'a` で表現されるため trait param 側は不要。
pub trait Element {
    /// HTML / XML tag name (例: `"p"`, `"div"`)。
    fn tag_name(&self) -> &str;

    /// HTML `style="..."` attribute の生 string を返す。
    /// 未設定または該当 attribute が空文字列 (`style=""`) の場合 `None`。
    ///
    /// raikiri-style::cascade が cssparser の declaration-list parser でこの
    /// 文字列を消費する (M1.4)。`self.attr("style")` の shorthand として
    /// 別 method を維持。
    ///
    /// Default impl は `None` — style を持たない Node kind や未対応 impl は
    /// override 不要。
    fn inline_style_source(&self) -> Option<&str> {
        None
    }

    /// Element の namespace URI (例: `"http://www.w3.org/2000/svg"`)。
    /// HTML default namespace の element は `None` を返す (fast path)。
    ///
    /// html5ever の `QualName.ns` (interned URI) から raikiri-html sink が
    /// SmolStr に写し取り、raikiri-dom::Node に格納する。
    fn namespace_uri(&self) -> Option<&str> {
        None
    }

    /// `id` attribute の値。空文字列 `id=""` は `None` を返す (attr lookup と
    /// 同じ boundary)。複数 token は spec 上 invalid だが raw value をそのまま
    /// 返す (tokenize しない)。
    ///
    /// Default impl は [`Element::attr`]`("id")` に delegate。impl 側は attr
    /// だけ override すれば id() も追従する (DRY / consistency 契約)。
    fn id(&self) -> Option<&str> {
        self.attr("id")
    }

    /// `class` attribute (space-separated) に指定 token が含まれているか。
    /// HTML spec に従い ASCII whitespace (space, tab, LF, CR, FF) で split。
    /// `class` attribute 自体が無い / 空 / 該当 token 無しは `false`。
    ///
    /// Default impl は [`Element::attr`]`("class")` を token 化して判定。
    /// impl 側は attr だけ override すれば has_class も追従する。
    fn has_class(&self, class: &str) -> bool {
        if class.is_empty() {
            return false;
        }
        self.attr("class").is_some_and(|value| {
            value
                .split([' ', '\t', '\n', '\r', '\x0C'])
                .any(|token| token == class)
        })
    }

    /// null-namespace attribute の value を local name で lookup。
    /// 未設定または空文字列は `None` を返す (spec 上 attribute 有無と empty
    /// value を区別する scenario が cascade / selector には無いため boundary
    /// で正規化)。
    ///
    /// `style` を渡した場合の返り値は [`Element::inline_style_source`] と一致
    /// する (両者は同じ side を参照する view)。Default impl は "style" のみ
    /// [`inline_style_source`](Element::inline_style_source) を返し、他は
    /// `None`。impl 側で `attr` を override する場合も "style" 特別扱いを
    /// 忘れないよう `self.inline_style_source()` へ redirect すること。
    fn attr(&self, local: &str) -> Option<&str> {
        if local == "style" {
            self.inline_style_source()
        } else {
            None
        }
    }
}

/// HTML5 quirks mode. m1.3 で raikiri-html が set し、UncascadedDocument
/// を経由して cascade (m1.4) が参照する。html5ever `QuirksMode` の raikiri
/// 面ミラー (cleanroom: html5ever を trait layer に持ち込まない)。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QuirksMode {
    /// Standards mode (`<!DOCTYPE html>` 明示、または QuirksMode 判定に該当せず)。
    #[default]
    NoQuirks,
    /// Limited quirks mode。
    LimitedQuirks,
    /// Full quirks mode (missing / obsolete DOCTYPE)。
    Quirks,
}

// ─────────────────────────────────────────────────────────────
// StylesheetKind — Document に associate される stylesheet の kind。
// (M1.4a、raikiri-spike-m1.22)
// ─────────────────────────────────────────────────────────────

/// Document に associate される stylesheet の kind。
///
/// CSS Cascading L4 §6.2 の origin concept を dom-level に反映するための
/// nominal tag。cascade phase (raikiri umbrella crate) で
/// `raikiri_style::Origin` にマップされる。
///
/// M1 では `UserAgent` + `Author` の 2 段のみ。User origin は Consumer が
/// `extra_stylesheets` (Author 扱い) で提供する想定 (spec §M1.4a Non-goals)。
/// 将来必要になったら variant を追加 (`#[non_exhaustive]` で non-breaking)。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StylesheetKind {
    /// User Agent origin (bundled minimal UA CSS 等)。cascade 内で最弱、
    /// ただし `!important` の場合は最強 (CSS Cascading L4 §6.4.4 反転扱い)。
    UserAgent,
    /// Author origin (HTML `<style>` element、`<link rel=stylesheet>`、
    /// Consumer 提供の `extra_stylesheets` 等)。M1 では User origin を
    /// Author に混ぜて扱う。
    Author,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// `Symbol` の `Ord` / `PartialOrd` 実装が SmolStr (= &str) 由来の
    /// lexicographic order を継承していることを確認する。M4 target-* /
    /// fragment-id で `BTreeMap<Symbol, _>` の deterministic iteration
    /// order を根拠にした logic を書く前提の unit contract。
    #[test]
    fn symbol_ord_matches_str_lexicographic() {
        // 2-element comparison: "a" < "b" (str lexicographic)。
        let a = Symbol::from("a");
        let b = Symbol::from("b");
        assert!(a < b);
        assert_eq!(a.cmp(&b), std::cmp::Ordering::Less);

        // BTreeSet insertion 順序に依らず iteration が sorted order で走る。
        let mut set = BTreeSet::new();
        set.insert(Symbol::from("charlie"));
        set.insert(Symbol::from("alpha"));
        set.insert(Symbol::from("bravo"));
        let collected: Vec<&str> = set.iter().map(Symbol::as_str).collect();
        assert_eq!(collected, ["alpha", "bravo", "charlie"]);
    }

    /// raikiri-spike-84y: `NodeKind` に追加された `Comment` /
    /// `ProcessingInstruction` / `DocumentFragment` variant が pattern-match
    /// で discriminate 可能かつ `Element` と PartialEq で区別できることを
    /// pin する (Two-way invariant の trait 側 constraint)。
    #[test]
    fn node_kind_variants_are_distinct_and_matchable() {
        for kind in [
            NodeKind::Element,
            NodeKind::Text,
            NodeKind::Document,
            NodeKind::Comment,
            NodeKind::ProcessingInstruction,
            NodeKind::DocumentFragment,
        ] {
            // 84y 3 variant はすべて Element とは PartialEq 上区別される。
            if !matches!(kind, NodeKind::Element) {
                assert_ne!(
                    kind,
                    NodeKind::Element,
                    "{kind:?} must not compare equal to Element"
                );
            }
        }
        // discriminant 一致性: 同じ variant 同士は equal。
        assert_eq!(NodeKind::Comment, NodeKind::Comment);
        assert_eq!(
            NodeKind::ProcessingInstruction,
            NodeKind::ProcessingInstruction
        );
        assert_eq!(NodeKind::DocumentFragment, NodeKind::DocumentFragment);
    }
}
