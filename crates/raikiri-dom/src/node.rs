//! Arena node type for raikiri-dom's Document.
//!
//! Node は Element / Text / Document (root) の 3 種を union で表現する
//! flat struct。fields は crate-private (raikiri-dom 内部のみ mutate)、
//! 外部 Consumer は `raikiri_traits::Dom / Node / Element` trait 経由で
//! 参照する。

use smol_str::SmolStr;
use taffy::{Cache, Layout, Style};

use raikiri_traits::NodeKind;

bitflags::bitflags! {
    /// Node に付随する per-node boolean 属性。blitz `NodeFlags` と bit 位置
    /// 1:1 対応 (M6 blitz-compat の nominal 変換前提)。
    ///
    /// M1 spike では `IS_IN_DOCUMENT` のみ定義。将来 `IS_INLINE_ROOT` (M3
    /// inline formatting root)、`IS_TABLE_ROOT` (M3+ table formatting root)
    /// を blitz と同 bit 位置で追加する予定。
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct NodeFlags: u32 {
        /// この Node が flat tree に含まれるか。`<template>` element の子孫は
        /// clear、Document root から flat-tree-parent 経由で到達可能な node は
        /// set。将来 shadow DOM / slot の "shadow-including tree" 意味論を
        /// 追加する場合、slot 割当てられない host 直下や shadow root 外の
        /// light-DOM 子孫も同 bit で表現する予定。
        ///
        /// 維持タイミング:
        /// - parse: `raikiri-html::sink::finish` の `mark_in_document_flags`
        ///   phase で single-pass DFS が set/clear
        /// - mutation runtime (M2+): mutator の `process_added_subtree` /
        ///   `process_removed_subtree` 相当が set/unset
        const IS_IN_DOCUMENT = 1 << 0;
    }
}

/// Element attribute (null namespace only for M1)。
///
/// namespaced attribute (`xlink:href` on SVG 等) は M2+ に defer。html5ever の
/// `Attribute.name.ns` が null namespace (`ns!("")`) の attr のみここに格納する。
/// `raikiri-html::sink::finish` が side-table から wire する。
#[derive(Debug, Clone)]
pub(crate) struct Attr {
    pub(crate) local: SmolStr,
    pub(crate) value: SmolStr,
}

/// NodeData: kind 固有 field を集約した tagged union (raikiri-spike-37c)。
///
/// blitz `blitz-dom::node::node::NodeData` に対応する shape。M6 blitz-compat
/// で nominal 変換 (`match data { NodeData::Element(e) => BlitzElement { ... }, ... }`)
/// できるように field 名を揃える。`Element` variant のみ `Box` で indirection
/// を挟むのは blitz と同じ選択 (Element の field 数が多く、Text / Document 側の
/// サイズに Element を引きずられさせないため)。
///
/// 注意: この Box は `size_of::<NodeData>()` を小さく抑えるものではない —
/// `TextData` が `parley::Layout<()>` を直接持つため `Element` variant
/// (Box 経由でポインタ幅) より大きく、結局 enum 全体は `TextData` のサイズで
/// 決まる (`clippy::large_enum_variant` が発火するのはこのため)。それでも
/// `Text` を Box しないのは意図した trade-off: `text_layout()` は paint hot
/// path から呼ばれるため、追加の indirection を持ち込みたくない。
#[derive(Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "Element only is boxed by design (blitz-compat shape, see doc comment); \
              Text carries parley::Layout<()> inline to avoid extra indirection on \
              the paint hot path"
)]
pub enum NodeData {
    /// HTML / XML element (tag_name + attributes + namespace + inline_style +
    /// template_contents slot を持つ)。
    Element(Box<ElementData>),
    /// Character data node。
    Text(TextData),
    /// Document root (arena index 0 の virtual node)。
    Document,
}

impl NodeData {
    /// Element variant を crate-private に mut borrow (Document setter 用)。
    #[inline]
    pub(crate) fn as_element_mut(&mut self) -> Option<&mut ElementData> {
        match self {
            NodeData::Element(e) => Some(e.as_mut()),
            _ => None,
        }
    }

    /// Text variant を crate-private に mut borrow (layout::preshape_text 用)。
    #[inline]
    pub(crate) fn as_text_mut(&mut self) -> Option<&mut TextData> {
        match self {
            NodeData::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// Element-only data (raikiri-spike-37c)。blitz `ElementData` に対応。
///
/// `template_contents` は `<template>` element の contents fragment root への
/// arena index を保持する slot として予約。M1 spike では sink が populate せず
/// `get_template_contents` は `*target` を返す (blitz と同じ TODO 状態)。M2+ で
/// clone/inject fixture が必要になった時に populate する
/// (raikiri-spike-xno Part 2)。
#[derive(Debug)]
pub struct ElementData {
    /// HTML / XML tag name (例: `"p"`, `"div"`)。html5ever の QualName.local から
    /// SmolStr に写し取る。
    pub(crate) tag_name: SmolStr,
    /// HTML `style="..."` attribute の生 string (kind == Element 時のみ populate、
    /// 空文字列 `style=""` は Element trait contract 上 `None` として view 化
    /// されるが、storage はここでは正規化せず raw 値を持つ)。
    pub(crate) inline_style: Option<SmolStr>,
    /// Element namespace URI (non-HTML の場合のみ `Some`、HTML default は
    /// `None` を fast path とする)。例: `Some("http://www.w3.org/2000/svg")`。
    pub(crate) namespace: Option<SmolStr>,
    /// null-namespace attribute list (順序保持、cascade tie-breaking で使う想定)。
    /// `style` attribute は [`ElementData::inline_style`] に分離済のためここには
    /// 含めない。
    pub(crate) attributes: Vec<Attr>,
    /// `<template>` element の contents fragment root への arena index。
    ///
    /// M1 spike では sink が populate しない (常に `None`)。`get_template_contents`
    /// も `*target` を返し続ける。M2+ で raikiri-spike-xno Part 2 の中で
    /// populate 実装 + `get_template_contents` の切り替えを行う。blitz
    /// `blitz-dom::node::element::ElementData::template_contents` と同名・同 shape。
    #[allow(dead_code, reason = "reserved for raikiri-spike-xno Part 2")]
    pub(crate) template_contents: Option<usize>,
}

/// Text-only data (raikiri-spike-37c)。blitz `TextNodeData` (nominally) に対応。
#[derive(Debug)]
pub struct TextData {
    /// Character data。
    pub(crate) text_content: SmolStr,
    /// Text node の pre-shaped parley Layout。
    ///
    /// - Populated by [`crate::layout::preshape_text`] (M1.6)
    /// - Consumed by taffy leaf measure closure (intrinsic size) と m1.7 paint
    ///   (glyph 位置)
    /// - Brush type `()` は M1.6 の choice: color / decoration は持たせない
    /// - Invalidation: `layout_single_page` 呼び出し毎に全 None にクリア + 再走
    pub text_layout: Option<parley::Layout<()>>,
}

/// Arena node (raikiri-spike-37c refactor: NodeData tagged union に移行)。
///
/// paint / cascade / layout に必要な kind 非依存の field (children /
/// unrounded_layout) は Node に残し、kind 固有 field は [`NodeData`] variant
/// に集約する。raikiri-spike-m1.7 で pub 化した 5 field のうち `kind` /
/// `tag_name` / `text_layout` は accessor method 経由に移行 (`node.kind()` /
/// `node.tag_name()` / `node.text_layout()`)、`children` / `unrounded_layout`
/// は pub field 継続。M1.15 external contract は Node/Element field access 0
/// 件なので無影響、raikiri-dom 内部 pub_surface pin のみ accessor 経由に再 pin。
#[derive(Debug)]
pub struct Node {
    /// Taffy layout style。
    pub(crate) style: Style,
    /// Child arena indices (`Document::nodes` の usize)。
    pub children: Vec<usize>,
    /// Taffy layout cache (per-node)。
    pub(crate) cache: Cache,
    /// Taffy layout 結果 (compute_root_layout が populate)。
    pub unrounded_layout: Layout,
    /// Per-node metadata bits (IS_IN_DOCUMENT etc.)。crate-private mutation。
    pub(crate) flags: NodeFlags,
    /// Node kind + kind 固有 field (tagged union)。
    pub(crate) data: NodeData,
}

impl Node {
    /// Document root node (arena index 0 用) を構築する。
    pub(crate) fn new_document() -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Document,
        }
    }

    /// Element node を tag name / style / inline_style と共に構築する。
    /// `namespace` / `attributes` / `template_contents` は初期空/None で、raikiri-html
    /// sink が finish 時に [`crate::Document::set_element_namespace`] /
    /// [`crate::Document::set_element_attributes`] で populate する
    /// (template_contents は M1 では populate なし)。
    pub(crate) fn new_element(tag: SmolStr, style: Style, inline_style: Option<SmolStr>) -> Self {
        Self {
            style,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Element(Box::new(ElementData {
                tag_name: tag,
                inline_style,
                namespace: None,
                attributes: Vec::new(),
                template_contents: None,
            })),
        }
    }

    /// Text node を character data と共に構築する。
    pub(crate) fn new_text(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Text(TextData {
                text_content: text,
                text_layout: None,
            }),
        }
    }

    // ─── inherent accessor methods (raikiri-spike-37c) ─────────────────

    /// この Node の [`NodeKind`] を返す。
    ///
    /// 旧 `pub kind: NodeKind` field の accessor 版 (raikiri-spike-37c refactor)。
    /// 呼び出し側は `node.kind` → `node.kind()` の syntax 変更のみ。
    #[inline]
    pub fn kind(&self) -> NodeKind {
        match &self.data {
            NodeData::Element(_) => NodeKind::Element,
            NodeData::Text(_) => NodeKind::Text,
            NodeData::Document => NodeKind::Document,
        }
    }

    /// Element の場合 tag_name を、それ以外は `None` を返す。
    ///
    /// 旧 `pub tag_name: Option<SmolStr>` field の accessor 版
    /// (raikiri-spike-37c refactor)。`Option<&str>` に射影する
    /// (SmolStr の内部 view で Copy 相当のコスト)。
    #[inline]
    pub fn tag_name(&self) -> Option<&str> {
        match &self.data {
            NodeData::Element(e) => Some(e.tag_name.as_str()),
            _ => None,
        }
    }

    /// Text の場合 text_layout を、それ以外は `None` を返す。
    ///
    /// 旧 `pub text_layout: Option<parley::Layout<()>>` field の accessor 版
    /// (raikiri-spike-37c refactor)。paint hot path から呼ばれるため `#[inline]`。
    #[inline]
    pub fn text_layout(&self) -> Option<&parley::Layout<()>> {
        match &self.data {
            NodeData::Text(t) => t.text_layout.as_ref(),
            _ => None,
        }
    }

    /// このノードの `taffy::Style.display == Display::None` を返す。
    ///
    /// paint 段で display:none subtree を skip する目的の predicate。size 0
    /// による代理判定は overflow: visible の legitimate な zero-size 要素を
    /// silent drop するため誤り (roborev job 223 finding 対応)。style field
    /// は crate-private のまま維持し、paint に必要な最小の boolean 述語のみ
    /// pub で公開する (gradual exposure)。
    #[inline]
    pub fn is_display_none(&self) -> bool {
        self.style.display == taffy::Display::None
    }

    /// この Node が flat tree の一員かを返す (raikiri-spike-37c)。
    #[inline]
    pub fn is_in_document(&self) -> bool {
        self.flags.contains(NodeFlags::IS_IN_DOCUMENT)
    }

    /// [`NodeFlags::IS_IN_DOCUMENT`] bit を明示的に上書きする (crate-private)。
    ///
    /// raikiri-spike-37c: `Document::mark_in_document_flags` (sink.finish() から
    /// 呼ばれる single-pass DFS) が消費する。
    #[inline]
    pub(crate) fn set_in_document(&mut self, v: bool) {
        self.flags.set(NodeFlags::IS_IN_DOCUMENT, v);
    }
}

#[cfg(test)]
mod flags_tests {
    use super::*;

    #[test]
    fn node_flags_default_is_empty() {
        let f = NodeFlags::default();
        assert!(!f.contains(NodeFlags::IS_IN_DOCUMENT));
    }

    #[test]
    fn node_new_document_has_is_in_document_set_by_default() {
        // Node::new_document() は Document root 用、常に flat tree の一員。
        let n = Node::new_document();
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_element_has_is_in_document_set_by_default() {
        let n = Node::new_element(SmolStr::new("p"), taffy::Style::default(), None);
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_text_has_is_in_document_set_by_default() {
        let n = Node::new_text(SmolStr::new("hi"));
        assert!(n.is_in_document());
    }

    #[test]
    fn set_in_document_toggles_bit() {
        let mut n = Node::new_document();
        n.set_in_document(false);
        assert!(!n.is_in_document());
        n.set_in_document(true);
        assert!(n.is_in_document());
    }
}
