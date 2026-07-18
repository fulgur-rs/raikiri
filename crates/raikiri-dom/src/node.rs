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

/// Arena node。paint に必要な 5 field は pub、他は crate-private (gradual
/// exposure)。M4 で cascade property 追加時に必要分を pub 化する。
///
/// M1.5 では `style` を Consumer が taffy::Style 直接構築する形。M1.6
/// layout-single-page で ComputedValues → taffy::Style 変換 layer が入る予定。
/// M1.4 で `inline_style` field を追加 (HTML `style="..."` 属性の生 string を保持、
/// raikiri-style::cascade が declaration-list として parse する)。
/// raikiri-spike-blg で `namespace` / `attributes` field を追加
/// (raikiri-html sink が finish 時に side-table から wire)。
/// raikiri-spike-m1.7 で `Node` を pub struct に昇格、paint に必要な field 5 個
/// (children / unrounded_layout / kind / tag_name / text_layout) を pub 化。
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
    /// Per-node metadata bits (raikiri-spike-37c)。IS_IN_DOCUMENT etc.
    ///
    /// crate-private: mutation は Document 経由 (`set_element_*` / sink の
    /// `mark_in_document_flags` phase) で行う。参照は [`Node::is_in_document`]
    /// 等の inherent accessor 経由。
    pub(crate) flags: NodeFlags,
    /// Node kind (Element / Text / Document)。
    pub kind: NodeKind,
    /// Element tag name (kind == Element 時のみ populate、他は `None`)。
    pub tag_name: Option<SmolStr>,
    /// Text character data (kind == Text 時のみ populate、他は `None`)。
    pub(crate) text_content: Option<SmolStr>,
    /// HTML `style="..."` attribute の生 string (kind == Element 時のみ populate、
    /// 他は `None`)。M1.4 raikiri-style::cascade が消費。
    pub(crate) inline_style: Option<SmolStr>,
    /// Element namespace URI (kind == Element かつ non-HTML の場合のみ `Some`、
    /// HTML default namespace は `None` を fast path とする)。
    /// 例: `Some("http://www.w3.org/2000/svg")`。
    pub(crate) namespace: Option<SmolStr>,
    /// null-namespace attribute list (kind == Element 時のみ populate、他は空)。
    /// 順序保持 (html5ever の source order、cascade tie-breaking で使う想定)。
    /// `style` attribute は [`Node::inline_style`] に分離済のためここには含めない。
    pub(crate) attributes: Vec<Attr>,
    /// Text node の pre-shaped parley Layout。Element / Document は常に None。
    ///
    /// - Populated by [`crate::layout::preshape_text`] (M1.6)
    /// - Consumed by taffy leaf measure closure (intrinsic size) と m1.7 paint
    ///   (glyph 位置)
    /// - Brush type `()` は M1.6 の choice: color / decoration は持たせない
    ///   (paint 段で ComputedValues.color を別途拾う)。M3 で `peniko::Brush`
    ///   等に昇格予定
    /// - Invalidation: `layout_single_page` 呼び出し毎に全 None にクリア +
    ///   再走。granular invalidation は M2+
    pub text_layout: Option<parley::Layout<()>>,
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
            kind: NodeKind::Document,
            tag_name: None,
            text_content: None,
            inline_style: None,
            namespace: None,
            attributes: Vec::new(),
            text_layout: None,
        }
    }

    /// Element node を tag name / style / inline_style と共に構築する。
    /// `namespace` / `attributes` は初期空で、raikiri-html sink が finish 時に
    /// [`crate::Document::set_element_namespace`] / [`crate::Document::set_element_attributes`]
    /// で populate する。
    pub(crate) fn new_element(tag: SmolStr, style: Style, inline_style: Option<SmolStr>) -> Self {
        Self {
            style,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            kind: NodeKind::Element,
            tag_name: Some(tag),
            text_content: None,
            inline_style,
            namespace: None,
            attributes: Vec::new(),
            text_layout: None,
        }
    }

    /// この Node が flat tree の一員かを返す (raikiri-spike-37c)。
    ///
    /// [`NodeFlags::IS_IN_DOCUMENT`] bit のシンプルな view。詳細は
    /// [`NodeFlags::IS_IN_DOCUMENT`] の doc を参照。
    #[inline]
    pub fn is_in_document(&self) -> bool {
        self.flags.contains(NodeFlags::IS_IN_DOCUMENT)
    }

    /// [`NodeFlags::IS_IN_DOCUMENT`] bit を明示的に上書きする (crate-private)。
    ///
    /// sink の `mark_in_document_flags` phase および将来の mutation runtime が
    /// 呼ぶ。外部 consumer が直接触ることは無い。
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn set_in_document(&mut self, v: bool) {
        self.flags.set(NodeFlags::IS_IN_DOCUMENT, v);
    }

    /// このノードの `taffy::Style.display == Display::None` を返す。
    ///
    /// paint 段で display:none subtree を skip する目的の predicate。size 0
    /// による代理判定は overflow: visible の legitimate な zero-size 要素を
    /// silent drop するため誤り (roborev job 223 finding 対応)。style field
    /// は crate-private のまま維持し、paint に必要な最小の boolean 述語のみ
    /// pub で公開する (gradual exposure)。
    pub fn is_display_none(&self) -> bool {
        self.style.display == taffy::Display::None
    }

    /// Text node を character data と共に構築する。
    pub(crate) fn new_text(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            kind: NodeKind::Text,
            tag_name: None,
            text_content: Some(text),
            inline_style: None,
            namespace: None,
            attributes: Vec::new(),
            text_layout: None,
        }
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
