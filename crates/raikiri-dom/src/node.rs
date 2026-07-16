//! Arena node type for raikiri-dom's Document.
//!
//! Node は Element / Text / Document (root) の 3 種を union で表現する
//! flat struct。fields は crate-private (raikiri-dom 内部のみ mutate)、
//! 外部 Consumer は `raikiri_traits::Dom / Node / Element` trait 経由で
//! 参照する。

use smol_str::SmolStr;
use taffy::{Cache, Layout, Style};

use raikiri_traits::NodeKind;

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

/// Arena node。全 field は crate-private。
///
/// M1.5 では `style` を Consumer が taffy::Style 直接構築する形。M1.6
/// layout-single-page で ComputedValues → taffy::Style 変換 layer が入る予定。
/// M1.4 で `inline_style` field を追加 (HTML `style="..."` 属性の生 string を保持、
/// raikiri-style::cascade が declaration-list として parse する)。
/// raikiri-spike-blg で `namespace` / `attributes` field を追加
/// (raikiri-html sink が finish 時に side-table から wire)。
#[derive(Debug)]
pub(crate) struct Node {
    /// Taffy layout style。
    pub(crate) style: Style,
    /// Child arena indices (`Document::nodes` の usize)。
    pub(crate) children: Vec<usize>,
    /// Taffy layout cache (per-node)。
    pub(crate) cache: Cache,
    /// Taffy layout 結果 (compute_root_layout が populate)。
    pub(crate) unrounded_layout: Layout,
    /// Node kind (Element / Text / Document)。
    pub(crate) kind: NodeKind,
    /// Element tag name (kind == Element 時のみ populate、他は `None`)。
    pub(crate) tag_name: Option<SmolStr>,
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
    pub(crate) text_layout: Option<parley::Layout<()>>,
}

impl Node {
    /// Document root node (arena index 0 用) を構築する。
    pub(crate) fn new_document() -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
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
    pub(crate) fn new_element(
        tag: SmolStr,
        style: Style,
        inline_style: Option<SmolStr>,
    ) -> Self {
        Self {
            style,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            kind: NodeKind::Element,
            tag_name: Some(tag),
            text_content: None,
            inline_style,
            namespace: None,
            attributes: Vec::new(),
            text_layout: None,
        }
    }

    /// Text node を character data と共に構築する。
    pub(crate) fn new_text(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
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
