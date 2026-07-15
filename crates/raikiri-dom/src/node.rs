//! Arena node type for raikiri-dom's Document.
//!
//! Node は Element / Text / Document (root) の 3 種を union で表現する
//! flat struct。fields は crate-private (raikiri-dom 内部のみ mutate)、
//! 外部 Consumer は `raikiri_traits::Dom / Node / Element` trait 経由で
//! 参照する。

use smol_str::SmolStr;
use taffy::{Cache, Layout, Style};

use raikiri_traits::NodeKind;

/// Arena node。全 field は crate-private。
///
/// M1.5 では `style` を Consumer が taffy::Style 直接構築する形。M1.6
/// layout-single-page で ComputedValues → taffy::Style 変換 layer が入る予定。
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
        }
    }

    /// Element node を tag name と style と共に構築する。
    pub(crate) fn new_element(tag: SmolStr, style: Style) -> Self {
        Self {
            style,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            kind: NodeKind::Element,
            tag_name: Some(tag),
            text_content: None,
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
        }
    }
}
