//! DOM abstraction trait + identifier newtypes.
//!
//! `Dom` / `Element` / `Node` は今後 associated type + method を確定する予定。
//! 現時点では shell として trait だけ用意し、raikiri-dom 側の実装検討と
//! co-design する。

use smol_str::SmolStr;

/// String identifier for GCPM fragments / running templates / named strings /
/// target-* references.
///
/// SmolStr newtype で inline 最適化を効かせる。GCPM で本格利用開始予定。
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
/// raikiri-dom node arena の kind field と対応する。
/// `Comment` / `ProcessingInstruction` / `DocumentFragment` は WHATWG DOM
/// §4 で列挙された NodeType のうち paged-media rendering に関係する 3 種と
/// して追加された。`#[non_exhaustive]` により変更は non-breaking (existing
/// callers は wildcard arm または `matches!(_, Element)` 形式で match する
/// ため影響なし)。
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
    /// mutation API の対象としては存在する。
    Comment,
    /// Processing instruction node (`<?target data?>`、HTML では実質発生
    /// しないが XML / XHTML では有効)。target + data を保持する。
    ProcessingInstruction,
    /// Document fragment root (`<template>` の contents fragment root や
    /// createDocumentFragment 相当の detached subtree の virtual root)。
    /// arena 内に detached 状態で存在し、Document root からは reachable
    /// でない (mark_in_document_flags 後 `is_in_document() == false`)。
    /// `<template>` fragment root の shape 修正のため追加 (旧:
    /// `"#document-fragment"` pseudo-tag な Element)。
    DocumentFragment,
}

/// DOM tree abstraction。raikiri-dom / raikiri-style / raikiri-paint / raikiri
/// (umbrella) が消費する generic navigation interface。
///
/// **Object-safety**: GAT (`type NodeRef<'a>`) を含むため non-object-safe。
/// 現状は generic dispatch (`fn walk<D: Dom>(dom: &D)`) を前提。dyn 化が
/// 必要な場合 (blitz-compat 経由の runtime abstraction 等、将来対応) は
/// erased wrapper trait を別途用意する。
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
    /// を含む)。
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
/// **Lifetime elision**: 過去は `Node<'a>` の form を持って
/// いたが、method signature で `'a` を使用しないため drop された。
/// borrowed Node value 自体の lifetime は `Dom::NodeRef<'a>` の `'a` で
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
    /// `as_element().is_some()`)。
    fn as_element(&self) -> Option<Self::Element<'_>>;

    /// kind が Text の場合 character data。それ以外 (Element / Document /
    /// Comment / ProcessingInstruction / DocumentFragment) は `None`。
    /// Comment / PI が character data 相当を持つ場合でも本 method は Text
    /// variant のみを返す (kind 分岐で明示区別)。
    fn text_content(&self) -> Option<&str>;

    /// この Node が flat tree に含まれるかを返す。`<template>` element の子孫
    /// は `false`、Document root から flat-tree-parent 経由で到達可能な node
    /// は `true`。
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
/// `inline_style_source` / `namespace_uri` / `id` / `has_class` / `attr` が
/// 追加された。attribute lookup は null-namespace attr のみ (namespaced
/// attr = xlink:href 等は将来 defer)。
///
/// **Lifetime elision**: 過去は `Element<'a>` の form を
/// 持っていたが、method signature で `'a` を使用しないため drop された。
/// borrowed Element value 自体の lifetime は `Dom::ElementRef<'a>`
/// の `'a` で表現されるため trait param 側は不要。
pub trait Element {
    /// HTML / XML tag name (例: `"p"`, `"div"`)。
    fn tag_name(&self) -> &str;

    /// HTML `style="..."` attribute の生 string を返す。
    /// 未設定または該当 attribute が空文字列 (`style=""`) の場合 `None`。
    ///
    /// raikiri-style::cascade が cssparser の declaration-list parser でこの
    /// 文字列を消費する。`self.attr("style")` の shorthand として
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

    /// `id` attribute の値。空文字列 `id=""` は `None` を返す — CSS
    /// Selectors L4 の ID selector (`#foo`) は空の ID token と一致しないため、
    /// この empty-is-absent 正規化は `id()` 固有の contract であり、
    /// [`Element::attr`] 自体の一般契約ではない (`attr()` は attribute の
    /// 有無と値を独立に追跡し、空文字列値でも `Some("")` を返す — 詳細は
    /// [`Element::attr`] の doc 参照)。複数 token は spec 上 invalid だが raw
    /// value をそのまま返す (tokenize しない)。
    ///
    /// Default impl は [`Element::attr`]`("id")` にそのまま delegate するため、
    /// `attr()` だけを override した impl では `id()` はこの empty-is-absent
    /// 正規化を自動的には満たさない。空 `id=""` を `None` として扱う必要が
    /// ある impl (例: `raikiri-dom::dom_impl::ElementRef`) は `id()` 自体を
    /// 個別に override すること。
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
    /// **attribute の有無と値は独立に追跡する**: 属性が未設定の場合のみ
    /// `None`、属性が存在すれば値が空文字列であっても `Some("")` を返す。
    /// CSS Selectors L4 の attribute-presence selector (`[foo]`) や
    /// exact-value selector (`[foo=""]`)、および HTML の boolean 属性
    /// (`disabled` / `open` / `hidden` 等) はいずれも値と無関係な「属性の
    /// 有無」で判定されるため、空文字列を absent と同一視してはならない。
    /// (`id` だけが持つ empty-is-absent 正規化は [`Element::id`] 側の個別
    /// contract であり、この一般 `attr()` には適用されない — 詳細は
    /// [`Element::id`] の doc 参照。)
    ///
    /// `style` を渡した場合の返り値は [`Element::inline_style_source`] と一致
    /// する (両者は同じ side を参照する view)。`inline_style_source` 自体は
    /// 「空の `style=""` は `None`」という別の独自 contract を持つ
    /// ([`Element::inline_style_source`] の doc 参照) — `style` はこの一般
    /// `attr()` の有無/値分離ルールの例外として、常に
    /// `inline_style_source()` へそのまま redirect される。Default impl は
    /// "style" のみ [`inline_style_source`](Element::inline_style_source) を
    /// 返し、他は (default 実装には attribute storage が無いため) 常に
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

/// HTML5 quirks mode. raikiri-html が set し、UncascadedDocument
/// を経由して cascade が参照する。html5ever `QuirksMode` の raikiri
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
// ─────────────────────────────────────────────────────────────

/// Document に associate される stylesheet の kind。
///
/// CSS Cascading L4 §6.2 の origin concept を dom-level に反映するための
/// nominal tag。cascade phase (raikiri umbrella crate) で
/// `raikiri_style::Origin` にマップされる。
///
/// 当初は `UserAgent` + `Author` の 2 段のみだった。`User` variant を追加し、
/// Consumer が `extra_stylesheets` 経由で提供する CSS を独立した user origin
/// として route できるようにした。旧実装は `Author` に混ぜて扱う暫定
/// (design doc の Non-goals として明記) だったが、将来 real author-origin
/// stylesheet (`<link rel=stylesheet>` 等) がこの経路に乗ってきたときに
/// 誤って user origin 扱いになる regression trap があったため、独立
/// variant として切り離した。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StylesheetKind {
    /// User Agent origin (bundled minimal UA CSS 等)。cascade 内で最弱、
    /// ただし `!important` の場合は最強 (CSS Cascading L4 §6.3 Importance
    /// <https://www.w3.org/TR/css-cascade-4/#importance> による origin 順の
    /// 反転。origin 自体の定義は §6.2
    /// <https://www.w3.org/TR/css-cascade-4/#cascading-origins>)。
    UserAgent,
    /// User origin (CSS Cascading L4 §6.2
    /// <https://www.w3.org/TR/css-cascade-4/#cascading-origins>)。Consumer
    /// が `ParseOptions::extra_stylesheets` 経由で提供する CSS はここに tag
    /// される (`raikiri-html/src/parse.rs`、`Author` から分離)。
    User,
    /// Author origin (HTML `<style>` element、`<link rel=stylesheet>` 等)。
    Author,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// `Symbol` の `Ord` / `PartialOrd` 実装が SmolStr (= &str) 由来の
    /// lexicographic order を継承していることを確認する。target-* /
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

    /// `NodeKind` に追加された `Comment` /
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
            // 追加された 3 variant はすべて Element とは PartialEq 上区別される。
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
