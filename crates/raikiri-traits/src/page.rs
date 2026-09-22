//! Page-related neutral model types.
//!
//! ここに集めた型は §5 (Pipeline), §7 (GCPM), §9 (PageBox), §11 (Paint) が
//! authoritative なので、現時点では opaque placeholder として置き、後続の
//! task が field / method を段階的に populate する。
//!
//! 入力/構築対象の struct は `#[non_exhaustive]` + `impl Default` + `pub fn new()` を持ち、
//! external consumer crate から `X::new()` / `X::default()` で construct 可能
//! ([`TargetInfo`] は `TargetRegistry::register` の input として consumer 側で構築)。
//! Output-only snapshot 型 (現状 [`PendingResolution`] のみ — registry 内部で
//! populate されて API 返り値経由で consumer に届くのみ) は consumer 側で直接
//! construct しないため Default / new を要件外とする
//! (`#[non_exhaustive]` は全 public struct に維持)。

mod context;
mod target;

pub use context::{CounterStack, NamedStringState, PageContext};
pub use target::{
    PendingResolution, ResolveOutcome, TargetInfo, TargetRegistry, resolve_content_component,
};

use std::collections::BTreeMap;

use crate::dom::{NodeId, Symbol};
use raikiri_style::property::{
    ContentComponent, ContentPart, ContentTextKeyword, CounterStyle, LeaderType, QuoteKeyword,
    StringFetchMode,
};
use raikiri_style::{Length, PageOrientation, PageSize, PageSizeKeyword};
#[cfg(test)]
use smol_str::SmolStr;
use url::Url;

/// One committed page of neutral layout geometry.
///
/// The producer is `raikiri-dom`'s pagination layer. The type deliberately
/// carries only CSS-px geometry and neutral identifiers; it does not expose
/// Taffy, Parley, `raikiri_style`, or a renderer-specific drawable payload.
/// `PageFragment` is the page-level container passed to a future streaming
/// sink. Its [`items`](Self::items) are ordered deterministically by the
/// pagination producer.
#[derive(Debug, Default, Clone, PartialEq)]
#[non_exhaustive]
pub struct PageFragment {
    /// Zero-based page index.
    pub page_index: u32,
    /// Resolved physical page box in CSS pixels.
    pub page_box: PageBox,
    /// Resolved page margins in CSS pixels.
    pub margins: PageFragmentInsets,
    /// Resolved border/padding inset inside the page margin area.
    pub content_insets: PageFragmentInsets,
    /// Effective page content rectangle in page-local CSS pixels.
    pub content_box: PageFragmentRect,
    /// The page's origin in the shared document body-content coordinate space.
    pub content_origin_y: f32,
    /// Resolved page name, if the page context selected one.
    pub page_name: Option<String>,
    /// Physical orientation derived from the resolved page box.
    pub orientation: PageFragmentOrientation,
    /// Per-node placements intersecting this page, in deterministic order.
    pub items: Vec<PageFragmentItem>,
}

impl PageFragment {
    /// Construct an empty page fragment snapshot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a page snapshot with resolved metadata and no items.
    #[allow(clippy::too_many_arguments)]
    pub fn with_metadata(
        page_index: u32,
        page_box: PageBox,
        margins: PageFragmentInsets,
        content_insets: PageFragmentInsets,
        content_box: PageFragmentRect,
        content_origin_y: f32,
        page_name: Option<String>,
        orientation: PageFragmentOrientation,
    ) -> Self {
        Self {
            page_index,
            page_box,
            margins,
            content_insets,
            content_box,
            content_origin_y,
            page_name,
            orientation,
            items: Vec::new(),
        }
    }

    /// Whether this page contains no visible node placements.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A CSS-px rectangle used by page-fragment metadata and placements.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PageFragmentRect {
    /// Left edge or x coordinate in CSS pixels.
    pub x: f32,
    /// Top edge or y coordinate in CSS pixels.
    pub y: f32,
    /// Width in CSS pixels.
    pub width: f32,
    /// Height in CSS pixels.
    pub height: f32,
}

impl PageFragmentRect {
    /// Construct a CSS-px rectangle.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Effective page margin or border/padding inset in CSS pixels.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PageFragmentInsets {
    /// Top inset.
    pub top: f32,
    /// Right inset.
    pub right: f32,
    /// Bottom inset.
    pub bottom: f32,
    /// Left inset.
    pub left: f32,
}

impl PageFragmentInsets {
    /// Construct four CSS-px insets.
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }
}

/// Physical orientation of a resolved page fragment.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageFragmentOrientation {
    /// Page width is less than or equal to page height.
    #[default]
    Portrait,
    /// Page width is greater than page height.
    Landscape,
}

/// Neutral kind classification for one page placement.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageFragmentKind {
    /// Element or container border-box placement.
    #[default]
    Box,
    /// Text-node placement.
    Text,
    /// Replaced-element placement such as an image.
    Replaced,
}

/// One source-node placement on a page.
///
/// This is the neutral counterpart to fulgur's `Fragment`: `page_index` is
/// zero-based and `rect` is expressed in CSS pixels in the page content-box
/// coordinate system. The containing [`PageFragment`] carries the resolved
/// page metadata; the duplicated index keeps node-centric geometry usable
/// without retaining the page-vector wrapper.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PageFragmentItem {
    /// Zero-based page index containing this placement.
    pub page_index: u32,
    /// Stable source DOM node identifier.
    pub node_id: NodeId,
    /// Fragment-local border-box rectangle in CSS pixels.
    pub rect: PageFragmentRect,
    /// Neutral placement classification.
    pub kind: PageFragmentKind,
    /// Zero-based fragment ordinal for this source node.
    pub fragment_index: u32,
    /// Total number of placements for this source node in the snapshot.
    pub fragment_count: u32,
    /// True when this placement repeats the complete source content.
    pub is_repeat: bool,
    /// Optional line range for a text placement (`start..end`, end exclusive).
    /// Non-text placements leave this as `None`.
    pub line_range: Option<PageFragmentLineRange>,
}

impl PageFragmentItem {
    /// Construct one page placement.
    pub fn new(
        node_id: NodeId,
        rect: PageFragmentRect,
        kind: PageFragmentKind,
        fragment_index: u32,
        fragment_count: u32,
        is_repeat: bool,
    ) -> Self {
        Self {
            page_index: 0,
            node_id,
            rect,
            kind,
            fragment_index,
            fragment_count,
            is_repeat,
            line_range: None,
        }
    }

    /// Set the zero-based page containing this placement.
    pub fn with_page_index(mut self, page_index: u32) -> Self {
        self.page_index = page_index;
        self
    }

    /// Attach a line range to a text placement.
    pub fn with_line_range(mut self, line_range: PageFragmentLineRange) -> Self {
        self.line_range = Some(line_range);
        self
    }

    /// Whether this node is split across multiple non-repeated placements.
    pub fn is_split(&self) -> bool {
        !self.is_repeat && self.fragment_count > 1
    }
}

/// Node-centric geometry collected from page snapshots.
///
/// This is the neutral counterpart to fulgur's `PaginationGeometry`: the
/// `fragments` vector is ordered by page and fragment index, while
/// `is_repeat` distinguishes complete per-page copies from split content. The
/// `raikiri-dom` producer emits repeat placements for fixed-position subtrees
/// when its existing layout produces the subtree geometry. Table header/footer
/// repetition is not synthesized until pagination owns a corresponding
/// repeated placement; that unsupported semantic remains explicit.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PageFragmentGeometry {
    /// Stable source DOM node identifier.
    pub node_id: NodeId,
    /// Placements for this node in ascending page order.
    pub fragments: Vec<PageFragmentItem>,
    /// True when every placement repeats the complete source content.
    pub is_repeat: bool,
}

impl PageFragmentGeometry {
    /// Construct empty node-centric geometry.
    pub fn new(node_id: NodeId, is_repeat: bool) -> Self {
        Self {
            node_id,
            fragments: Vec::new(),
            is_repeat,
        }
    }

    /// Whether this node's placements represent split content.
    pub fn is_split(&self) -> bool {
        !self.is_repeat && self.fragments.len() > 1
    }
}

/// Deterministic NodeId-ordered page geometry table.
///
/// `BTreeMap` iteration yields the same stable source-node order as the
/// pagination producer, independent of DOM traversal implementation details.
pub type PageFragmentGeometryTable = BTreeMap<NodeId, PageFragmentGeometry>;

/// Inclusive/exclusive line range carried by a text page placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageFragmentLineRange {
    /// First line index, inclusive.
    pub start: u32,
    /// Last line index, exclusive.
    pub end: u32,
}

impl PageFragmentLineRange {
    /// Construct a line range.
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Whether the range contains no lines.
    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// PageBox — the concrete paper-size part of an `@page` result.
/// `width` / `height` および `A4` / `US_LETTER` const は実装済み。
/// Page margins and margin-box declaration bags are carried by the page
/// cascade/scene layers rather than this two-dimensional paper-size value.
///
/// **単位 = CSS px** (1 CSS px = 1/96 in in print context per CSS Values L4 §6.2
/// "Absolute Lengths" <https://www.w3.org/TR/css-values-4/#absolute-lengths>)。
/// `from_page_size` は cascaded `@page size` の CSS absolute units をこの
/// CSS-px shape に変換する。直接 `PageBox` を構築する consumer は引き続き
/// CSS px を渡す。
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PageBox {
    /// Page 幅 (CSS px)。
    pub width: f32,
    /// Page 高 (CSS px)。
    pub height: f32,
    // 将来 populate 予定:
    //   pub margins: Margins,
    //   pub margin_boxes: `[Option<MarginBox>; 16]`,
}

impl PageBox {
    /// A4 portrait: 210×297 mm = **793.70 × 1122.52 px** (@ 96 DPI anchor)。
    /// CSS Paged Media Level 3 §7 default size。
    pub const A4: PageBox = PageBox {
        width: 793.7008,   // 210mm × 96/25.4
        height: 1122.5197, // 297mm × 96/25.4
    };

    /// US Letter portrait: 8.5×11 in = **816 × 1056 px** ちょうど。
    pub const US_LETTER: PageBox = PageBox {
        width: 816.0,
        height: 1056.0,
    };

    /// Construct a `PageBox` = `A4`。`#[non_exhaustive]` の下でも安定した
    /// zero-arg constructor を残すため保持。
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a cascaded CSS `size` descriptor into a concrete page box.
    ///
    /// `None` and `auto` use the A4 fallback. Absolute CSS lengths use the
    /// CSS 96-DPI conversion; relative lengths use the initial 16px font size
    /// because page descriptors have no element font context at this boundary.
    /// Invalid or non-positive values also use the fallback.
    pub fn from_page_size(size: Option<PageSize>) -> Self {
        let fallback = Self::A4;
        let Some(size) = size else {
            return fallback;
        };

        let (mut width, mut height, orientation) = match size {
            PageSize::Auto => return fallback,
            PageSize::Lengths { width, height } => {
                let Some(width) = page_length_to_px(width) else {
                    return fallback;
                };
                let Some(height) = page_length_to_px(height) else {
                    return fallback;
                };
                (width, height, None)
            }
            PageSize::Named {
                keyword,
                orientation,
            } => {
                let (width, height) = match keyword {
                    Some(keyword) => named_page_size(keyword),
                    None => (fallback.width, fallback.height),
                };
                (width, height, orientation)
            }
            _ => return fallback, // cov:ignore: future non-exhaustive PageSize variant
        };

        (width, height) = apply_page_orientation(width, height, orientation);
        // Viewport-unit expansion can leave an integer CSS length a few
        // floating-point ulps away from that integer (for example
        // `100vw` -> `480.00003px`).  Normalize only this tiny neighborhood so
        // raster page dimensions do not grow by one pixel; genuine subpixel
        // page sizes remain untouched.
        let normalize_near_integer = |value: f32| {
            let rounded = value.round();
            if value.is_finite() && (value - rounded).abs() < 0.001 {
                rounded
            } else {
                value
            }
        };
        width = normalize_near_integer(width);
        height = normalize_near_integer(height);

        if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
            Self { width, height }
        } else {
            fallback // cov:ignore: conversion rejects non-finite and non-positive lengths earlier
        }
    }
}

fn apply_page_orientation(
    mut width: f32,
    mut height: f32,
    orientation: Option<PageOrientation>,
) -> (f32, f32) {
    match orientation {
        Some(PageOrientation::Landscape) if height > width => {
            std::mem::swap(&mut width, &mut height);
        }
        Some(PageOrientation::Portrait) if width > height => {
            std::mem::swap(&mut width, &mut height);
        }
        _ => {}
    }
    (width, height)
}

fn page_length_to_px(length: Length) -> Option<f32> {
    let px = match length {
        Length::Px(value) => value,
        Length::Pt(value) => value * 96.0 / 72.0,
        Length::Cm(value) => value * 96.0 / 2.54,
        Length::Mm(value) => value * 96.0 / 25.4,
        Length::Q(value) => value * 96.0 / 101.6,
        Length::In(value) => value * 96.0,
        Length::Pc(value) => value * 16.0,
        Length::Em(value) | Length::Rem(value) => value * 16.0,
        Length::Ex(value) | Length::Ch(value) => value * 8.0,
        Length::Ic(value) => value * 16.0,
        Length::Rex(value) | Length::Rch(value) => value * 8.0,
        Length::Ric(value) => value * 16.0,
        Length::Lh(value) | Length::Rlh(value) => value * 16.0,
        Length::Percent(_) => return None,
        _ => return None, // cov:ignore: future non-exhaustive Length variant
    };
    (px.is_finite() && px > 0.0).then_some(px)
}

fn named_page_size(keyword: PageSizeKeyword) -> (f32, f32) {
    const MM: f32 = 96.0 / 25.4;
    match keyword {
        PageSizeKeyword::A5 => (148.0 * MM, 210.0 * MM),
        PageSizeKeyword::A4 => (210.0 * MM, 297.0 * MM),
        PageSizeKeyword::A3 => (297.0 * MM, 420.0 * MM),
        PageSizeKeyword::B5 => (176.0 * MM, 250.0 * MM),
        PageSizeKeyword::B4 => (250.0 * MM, 353.0 * MM),
        PageSizeKeyword::JisB5 => (182.0 * MM, 257.0 * MM),
        PageSizeKeyword::JisB4 => (257.0 * MM, 364.0 * MM),
        PageSizeKeyword::Letter => (8.5 * 96.0, 11.0 * 96.0),
        PageSizeKeyword::Legal => (8.5 * 96.0, 14.0 * 96.0),
        PageSizeKeyword::Ledger => (11.0 * 96.0, 17.0 * 96.0),
        _ => (PageBox::A4.width, PageBox::A4.height), // cov:ignore: future non-exhaustive PageSizeKeyword variant
    }
}

impl Default for PageBox {
    fn default() -> Self {
        Self::A4
    }
}

/// Consumer が render 開始時に渡す page-level default 値。現状は paper size
/// のみを持つ最小 shape。将来 margin / orientation / named pages 等を追加予定。
///
/// 全 field は CSS px 単位 (`PageBox` 参照)。pt/mm/in 換算は Consumer 責務。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageDefaults {
    /// Default paper サイズ (`@page size` で override しない場合の initial value)。
    /// 既定 = A4。
    pub page_box: PageBox,
}

impl PageDefaults {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> PageDefaultsBuilder {
        PageDefaultsBuilder::default()
    }
}

/// `PageDefaults` の fluent builder。
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct PageDefaultsBuilder {
    page_box: Option<PageBox>,
}

impl PageDefaultsBuilder {
    /// `page_box` を設定。
    pub fn page_box(mut self, v: PageBox) -> Self {
        self.page_box = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> PageDefaults {
        PageDefaults {
            page_box: self.page_box.unwrap_or_default(),
        }
    }
}

// `PageContext` — GCPM Phase B runtime state (counter tree, named string
// 4-snapshot, running bindings, target registry, page_index/page_name).
// design doc §7.2 canonical shape, promoted from an opaque placeholder onto
// the raikiri-dom-authored `PhaseBWalkState` algorithm (human-reviewed
// wall/traits crossing). Canonical impl is sibling `context` submodule; this
// module re-exports only.

/// LayoutBuffer — widow / orphan / break-inside / container probe lookahead
/// buffer の中立モデル。実装は raikiri-dom 側 (§5 参照)。
/// 未実装で、将来 populate される予定。
#[allow(missing_docs)]
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct LayoutBuffer {
    // 将来 populate 予定。
}

impl LayoutBuffer {
    /// Construct an empty LayoutBuffer (placeholder, not yet populated).
    pub fn new() -> Self {
        Self::default()
    }
}

// `TargetRegistry` — target-* placeholder emit + resolve の runtime registry。
// design doc §7.2 canonical shape、raikiri-dom 側の `pub(crate)` shadow 実装を
// 本 crate に merge したもの。canonical impl は sibling `target` submodule。
// この module では re-export のみ。

/// RunningTemplate — `position: running(name)` の template 登録。
/// 未実装で、将来 populate される予定。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct RunningTemplate {
    // 将来 populate 予定。
}

impl RunningTemplate {
    /// Construct an empty RunningTemplate (placeholder, not yet populated).
    pub fn new() -> Self {
        Self::default()
    }
}

/// FormData — application/x-www-form-urlencoded body の中立モデル。
/// Consumer 側 network 実装で参照 (Body::Form(FormData))。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct FormData {
    // 将来 populate:
    //   pub pairs: Vec<(String, String)>,
}

impl FormData {
    /// Construct an empty FormData (placeholder, not yet populated).
    pub fn new() -> Self {
        Self::default()
    }
}

/// Stable identifier for a running-element template registered via
/// `position: running(name)`.
///
/// design doc §7.0 line 1903-1905 "shared types → raikiri-traits" scope。
/// [`GcpmDirective::RegisterRunning`] (§7.1 line 1918) が field で参照する。
///
/// Wraps a subtree-root [`NodeId`]。`position: running(name)` された element の
/// subtree root は arena 内で per-element unique なので、そのまま stable な
/// per-template key として使える (canonical: raikiri-dom 内の
/// `RunningTemplateStore` の keying rationale と同一)。newtype で `NodeId` の
/// 他用途との mix を防ぐ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunningTemplateId(pub NodeId);

impl RunningTemplateId {
    /// Construct from a subtree-root [`NodeId`] (canonical: per-element unique).
    pub fn new(subtree_root: NodeId) -> Self {
        Self(subtree_root)
    }
}

/// Resolved `<content-list>` value used as the source of a `string-set` snapshot.
///
/// design doc §7.1 line 1917 `StringSet { name: Symbol, source: ContentSource }`。
/// raikiri-style cascade が `string-set: name <content-list>` declaration を
/// resolve し、raikiri-dom Phase B walk が §2.7.2 の 4-snapshot mode
/// (start / first / last / first-except) で [`PageContext`] named-string state
/// にコピーする際の source shape。
///
/// 将来、consumer (raikiri-style bridge) 側で [`Vec<ContentValueItem>`] から
/// 構築する。`items` は public field で `..Default::default()` の struct-update
/// syntax でも construct 可 (sibling [`PageBox`] pattern)。
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentSource {
    /// Ordered resolved items making up the content-list.
    pub items: Vec<ContentValueItem>,
}

impl ContentSource {
    /// Construct from a resolved item list.
    pub fn new(items: Vec<ContentValueItem>) -> Self {
        Self { items }
    }
}

/// GCPM producing directive emitted by raikiri-style cascade
/// (`counter-increment` / `counter-reset` / `counter-set` / `string-set` /
/// `position: running(name)` 等)、raikiri-dom Phase B walk が [`PageContext`]
/// と [`TargetRegistry`] を更新するために消費する (design doc §7.1 line
/// 1907-1920 — "Producing directive"、raikiri-style emit → raikiri-dom apply)。
///
/// **Producing side** of §7 GCPM: Phase B walk は running counter tree /
/// named-string 4-snapshot / running bindings / TargetRegistry を mutate する。
///
/// **`#[non_exhaustive]` semantics** — sibling [`ContentValueItem`] と同じ
/// forward-compat 契約: enum-level `#[non_exhaustive]` は downstream `match`
/// に `_ =>` arm を強制するが、既存 variant の tuple/struct constructor 呼び出しは
/// block しない。既存 variant の payload **type** 変更は downstream の
/// constructor を compile-break させる。
///
/// (design doc §7.1 line 1913-1920 の canonical 6 variant で、以前の
/// uninhabited placeholder を置き換え済み。)
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcpmDirective {
    /// `counter-increment: name delta` — 指定 counter `name` を `delta` だけ増分
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#propdef-counter-increment>)。
    CounterIncrement {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Increment amount (spec default 1、`counter-increment: name -3` で負値)。
        delta: i32,
    },
    /// `counter-reset: name value` — 指定 counter `name` を `value` に reset
    /// (CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#propdef-counter-reset>)。
    CounterReset {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Reset value (spec default 0)。
        value: i32,
    },
    /// `counter-set: name value` — 現要素で counter `name` を `value` に set
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#propdef-counter-set>)。
    CounterSet {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Set value (spec default 0)。
        value: i32,
    },
    /// `string-set: name <content-list>` — resolve 済 content-list を named-string
    /// `name` の 4-snapshot state (start / first / last / first-except) に snapshot
    /// (CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>)。
    StringSet {
        /// Named-string identifier。
        name: Symbol,
        /// Resolved source content-list ([`ContentSource`])。
        source: ContentSource,
    },
    /// `position: running(name)` — 現 subtree を running template として `name`
    /// 下に登録し、`template_id` (subtree root wrapped in [`RunningTemplateId`])
    /// を key として [`RunningTemplate`] pool に格納
    /// (CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>)。
    RegisterRunning {
        /// Running-template name (custom-ident)。
        name: Symbol,
        /// Template identifier (subtree root wrapped)。
        template_id: RunningTemplateId,
    },
    /// In-document fragment id を [`TargetRegistry`] に登録
    /// (`target-counter()` / `target-counters()` / `target-text()` の後続解決用)
    /// (design doc §7.2 TargetRegistry)。
    RegisterTarget {
        /// Fragment id (通常は element `id` attribute 値の Symbol view)。
        fragment_id: Symbol,
    },
}

/// resolved `content` property の item — [`GcpmDirective`] の "consuming"
/// counterpart (design doc §7.1 line 1923-1937 — "Consuming directive"、
/// raikiri-style cascade emit → raikiri-dom paint-time resolve で concrete
/// string に変換)。
///
/// **`#[non_exhaustive]` semantics** — sibling [`GcpmDirective`] および
/// [`raikiri_style::property::ContentComponent`] と同じ forward-compat 契約。
///
/// **`TargetCounters::sep`** — design doc §7.1 line 1935 の field 名は `sep`。
///   [`raikiri_style::property::ContentComponent::TargetCounters::separator`]
///   は `separator`。design doc に verbatim 従い `sep` を使う。[`TryFrom`] impl
///   で `separator → sep` を map。
///
/// # `Element` variant
///
/// `ContentValueItem::Element` is the design doc §7.1 line 1931
/// canonical representation of `element(name)`. The style parser and
/// bridge retain this name so the paint-side running-template resolver
/// can select the matching `position: running(name)` element.
///
/// (design doc §7.1 line 1926-1937 の canonical 10 variant で、以前の
/// uninhabited placeholder を置き換え済み。
///
/// [`Image`](Self::Image) / [`Contents`](Self::Contents) / [`Quote`](Self::Quote) /
/// [`Leader`](Self::Leader) の 4 variant は design doc §7.1 の canonical 10 には
/// **含まれない** — raikiri-style 側で `ContentComponent` に同 4 variant が
/// 追加されたこと (CSS Content 3 §2.2/§2.3/§2.4.2/§2.5.1) を受けた 1:1 mirror
/// 追加。[`QuoteKeyword`] / [`LeaderType`] は raikiri-style の型を直接 reuse、
/// sibling [`Counter`](Self::Counter) の `style: CounterStyle` 直接 reuse 慣行と
/// 同じ)。)
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentValueItem {
    /// bare `<string>` literal (`content: "hello"`)。
    Literal(String),
    /// `counter(name, style?)` (CSS Lists 3 §4.7
    /// <https://www.w3.org/TR/css-lists-3/#counter-functions>)。
    Counter {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Counter style (spec default `decimal`)。
        style: CounterStyle,
    },
    /// `counters(name, separator, style?)` (CSS Lists 3 §4.7)。
    Counters {
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Separator string (nested counter stack join)。
        separator: String,
        /// Counter style (spec default `decimal`)。
        style: CounterStyle,
    },
    /// `string(name, mode?)` (CSS Content 3 §2.7.2
    /// <https://www.w3.org/TR/css-content-3/#string-function>)。
    String {
        /// Named-string identifier。
        name: Symbol,
        /// Fetch mode (spec default `first`)。
        fetch: StringFetchMode,
    },
    /// `element(name)` (CSS GCPM 3 §1.2.2
    /// <https://www.w3.org/TR/css-gcpm-3/#element-syntax>)。runtime resolve は
    /// raikiri-dom の [`RunningTemplate`] pool 引きから。
    ///
    /// **From-impl gap**: `ContentComponent::Element` は未実装。
    /// See type-level docstring "Element variant" section。
    Element {
        /// Running-template name (custom-ident)。
        name: Symbol,
    },
    /// `content(part?)` (CSS GCPM 3 §1.1.1.1
    /// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content>) (`?` は raikiri の
    /// 受理済み記法であり spec の grammar 自体の表記ではない)。keyword 省略時は
    /// [`ContentPart::Content`] をフォールバック値として使う (根拠は spec の
    /// "default" 宣言ではない — 詳細は [`ContentTextKeyword`] の doc comment
    /// 参照)。
    Content {
        /// Element の string value のどの部分を挿入するか。
        part: ContentPart,
    },
    /// `attr(name)` (CSS Content 3 §2.1
    /// <https://www.w3.org/TR/css-content-3/#strings>)。
    Attr {
        /// Attribute name (null-namespace、CSS Content 3 §2.1)。
        name: Symbol,
    },
    /// `target-counter(url, name, style?)` (CSS Content 3 §2.6.1
    /// <https://www.w3.org/TR/css-content-3/#target-counter>)。
    TargetCounter {
        /// Target URL (bridge 側で [`Url::parse`] 済)。
        url: Url,
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Counter style (spec default `decimal`)。
        style: CounterStyle,
    },
    /// `target-counters(url, name, separator, style?)` (CSS Content 3 §2.6.2
    /// <https://www.w3.org/TR/css-content-3/#target-counters>)。
    ///
    /// field 名は design doc §7.1 line 1935 に verbatim (`sep`)。
    TargetCounters {
        /// Target URL (bridge 側で [`Url::parse`] 済)。
        url: Url,
        /// Counter name (custom-ident)。
        name: Symbol,
        /// Separator string (design doc verbatim `sep`)。
        sep: String,
        /// Counter style (spec default `decimal`)。
        style: CounterStyle,
    },
    /// `target-text(url, part?)` (CSS Content 3 §2.6.3
    /// <https://www.w3.org/TR/css-content-3/#target-text>)。
    TargetText {
        /// Target URL (bridge 側で [`Url::parse`] 済)。
        url: Url,
        /// どの部分を挿入するか (`content` は要素の string value 全体を指す
        /// keyword)。
        part: ContentPart,
    },
    /// `<image>` (`url()` alternative) — CSS Content 3 §2.2
    /// <https://www.w3.org/TR/css-content-3/#content-uri>。
    /// [`ContentComponent::Image`] の 1:1 mirror。
    ///
    /// **`<content-replacement>` 未実装**:
    /// [`raikiri_style::property::ContentComponent::Image`] の docstring と同じ
    /// 注意点がここにも及ぶ — 単一 `Image` item の `content-list` を
    /// `<content-replacement>` (pseudo-element 抑制 + 全要素置換) として扱う
    /// semantics は raikiri-dom Phase B / paint-time 責務で未実装。shape
    /// (`Vec<ContentValueItem>` の単一 `Image` 要素) 自体はこの区別を
    /// downstream が再構成するのに十分。
    Image {
        /// Image URL (bridge 側で [`Url::parse`] 済、sibling
        /// [`TargetCounter`](Self::TargetCounter) と同じ convention)。
        url: Url,
    },
    /// `contents` keyword — CSS Content 3 §2.3
    /// <https://www.w3.org/TR/css-content-3/#element-content>。
    /// [`ContentComponent::Contents`] の 1:1 mirror。
    Contents,
    /// `<quote>` (`open-quote` / `close-quote` / `no-open-quote` /
    /// `no-close-quote`) — CSS Content 3 §2.4.2
    /// <https://www.w3.org/TR/css-content-3/#quote-values>。
    /// [`ContentComponent::Quote`] の 1:1 mirror —
    /// payload は [`raikiri_style::property::QuoteKeyword`] を直接 reuse
    /// (traits-side 複製なし)。
    Quote(QuoteKeyword),
    /// `leader(<leader-type>)` — CSS Content 3 §2.5.1
    /// <https://www.w3.org/TR/css-content-3/#leader-function>。
    /// [`ContentComponent::Leader`] の 1:1 mirror —
    /// payload は [`raikiri_style::property::LeaderType`] を直接 reuse
    /// (traits-side 複製なし)。
    Leader(LeaderType),
}

/// [`TryFrom<ContentComponent> for ContentValueItem`] の failure taxonomy。
///
/// raikiri-style は `url` crate に依存しない leaf crate なので target-* variant
/// の URL は raw [`String`] で保持され、bridge 側で [`Url::parse`] する。
/// parse 失敗と、raikiri-style 側に新 variant が landing した際の
/// forward-compat 検知の 2 case を分離する。
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentValueConvertError {
    /// target-* variant の URL 文字列が [`Url::parse`] で fail した。
    InvalidUrl(url::ParseError),
    /// raikiri-style [`ContentComponent`] に本 crate 未対応 variant が新規追加された
    /// (cross-crate `#[non_exhaustive]` の catch-all)。
    ///
    /// stable Rust では cross-crate `#[non_exhaustive]` に対して compile-time
    /// 全 variant enforce できないため、runtime error でハンドリング。raikiri-style
    /// 側追加時に本 crate の [`TryFrom`] arm を extend する discipline。
    UnsupportedVariant,
}

impl core::fmt::Display for ContentValueConvertError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidUrl(e) => write!(f, "invalid URL in target-* content component: {e}"),
            Self::UnsupportedVariant => f.write_str(
                "unsupported ContentComponent variant (raikiri-style ahead of raikiri-traits, extend TryFrom arms)",
            ),
        }
    }
}

impl std::error::Error for ContentValueConvertError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidUrl(e) => Some(e),
            Self::UnsupportedVariant => None,
        }
    }
}

impl From<url::ParseError> for ContentValueConvertError {
    fn from(e: url::ParseError) -> Self {
        Self::InvalidUrl(e)
    }
}

/// [`ContentTextKeyword`] → [`ContentPart`] mapping (`content(keyword)` の
/// keyword を bridge)。keyword 集合は spec spelling が異なる: GCPM 3 §1.1.1.1
/// の `content()` と CSS Content 3 §2.6.3 の `target-text()` は同じ概念に
/// 異なる keyword spelling を当てている (`text` vs `content`)。両者を
/// [`ContentPart`] 側の value 集合に集約するときは意味の対応で紐付ける
/// (spec の "default" 宣言には依らない — CSS Content 3 §2.6.3 は
/// `target-text()` の第 2 引数省略時の値を規定していない。GCPM 3 §1.1.1.1
/// は `text` を "the default value" と述べているが、同 section は grammar に
/// `?` が無く [第 2 引数が構文上 optional でない]、かつ "default をどう
/// 定義するか" 自体が未解決の WG issue として残っており、TR 上安定した根拠
/// ではない):
/// - [`ContentTextKeyword::Text`] → [`ContentPart::Content`] (どちらも
///   「要素の string value 全体」を指す)
/// - [`ContentTextKeyword::Before`] → [`ContentPart::Before`]
/// - [`ContentTextKeyword::After`] → [`ContentPart::After`]
/// - [`ContentTextKeyword::FirstLetter`] → [`ContentPart::FirstLetter`]
fn content_text_keyword_to_content_part(
    kw: ContentTextKeyword,
) -> Result<ContentPart, ContentValueConvertError> {
    Ok(match kw {
        ContentTextKeyword::Text => ContentPart::Content,
        ContentTextKeyword::Before => ContentPart::Before,
        ContentTextKeyword::After => ContentPart::After,
        ContentTextKeyword::FirstLetter => ContentPart::FirstLetter,
        // cov:ignore: cross-crate `#[non_exhaustive]` catch-all — stable Rust
        // requires the `_` arm for exhaustive matching on ContentTextKeyword
        // defined in raikiri-style; unreachable until raikiri-style adds a
        // new variant. Treat it like any other unsupported cross-crate variant
        // rather than silently changing its semantics to the spec default.
        _ => return Err(ContentValueConvertError::UnsupportedVariant),
    })
}

impl TryFrom<ContentComponent> for ContentValueItem {
    type Error = ContentValueConvertError;

    /// [`raikiri_style::property::ContentComponent`] → [`ContentValueItem`]
    /// canonical taxonomy 変換。
    ///
    /// 変換対象 13 variant (Element は raikiri-style 側未実装、raikiri-style 側に
    /// 生えたら arm 追加):
    ///
    /// - [`ContentComponent::Literal`] → [`ContentValueItem::Literal`] (`String` に変換)
    /// - [`ContentComponent::Counter`] → [`ContentValueItem::Counter`]
    ///   (`name: SmolStr` → [`Symbol::new`])
    /// - [`ContentComponent::Counters`] → [`ContentValueItem::Counters`]
    /// - [`ContentComponent::String`] → [`ContentValueItem::String`]
    /// - [`ContentComponent::Attr`] → [`ContentValueItem::Attr`]
    /// - [`ContentComponent::TargetCounter`] → [`ContentValueItem::TargetCounter`]
    ///   (URL は [`Url::parse`]、失敗時 [`ContentValueConvertError::InvalidUrl`])
    /// - [`ContentComponent::TargetCounters`] → [`ContentValueItem::TargetCounters`]
    ///   (`separator` → `sep` field 名変換、design doc §7.1 line 1935 verbatim)
    /// - [`ContentComponent::TargetText`] → [`ContentValueItem::TargetText`]
    /// - [`ContentComponent::Content`] → [`ContentValueItem::Content`]
    ///   ([`ContentTextKeyword`] → [`ContentPart`] mapping)
    /// - [`ContentComponent::Image`] → [`ContentValueItem::Image`]
    ///   (URL は [`Url::parse`]、失敗時 [`ContentValueConvertError::InvalidUrl`])
    /// - [`ContentComponent::Contents`] → [`ContentValueItem::Contents`]
    ///   (unit variant 1:1)
    /// - [`ContentComponent::Quote`] → [`ContentValueItem::Quote`]
    ///   (payload [`QuoteKeyword`] straight passthrough)
    /// - [`ContentComponent::Leader`] → [`ContentValueItem::Leader`]
    ///   (payload [`LeaderType`] straight passthrough)
    fn try_from(cc: ContentComponent) -> Result<Self, Self::Error> {
        Ok(match cc {
            ContentComponent::Literal(s) => Self::Literal(s.into()),
            ContentComponent::Counter { name, style } => Self::Counter {
                name: Symbol::new(name),
                style,
            },
            ContentComponent::Counters {
                name,
                separator,
                style,
            } => Self::Counters {
                name: Symbol::new(name),
                separator,
                style,
            },
            ContentComponent::String { name, fetch } => Self::String {
                name: Symbol::new(name),
                fetch,
            },
            ContentComponent::Element { name } => Self::Element {
                name: Symbol::new(name),
            },
            ContentComponent::Attr { name } => Self::Attr {
                name: Symbol::new(name),
            },
            ContentComponent::TargetCounter { url, name, style } => Self::TargetCounter {
                url: Url::parse(&url)?,
                name: Symbol::new(name),
                style,
            },
            ContentComponent::TargetCounters {
                url,
                name,
                separator,
                style,
            } => Self::TargetCounters {
                url: Url::parse(&url)?,
                name: Symbol::new(name),
                sep: separator,
                style,
            },
            ContentComponent::TargetText { url, part } => Self::TargetText {
                url: Url::parse(&url)?,
                part,
            },
            ContentComponent::Content { keyword } => Self::Content {
                part: content_text_keyword_to_content_part(keyword)?,
            },
            ContentComponent::Image { url } => Self::Image {
                url: Url::parse(&url)?,
            },
            ContentComponent::Contents => Self::Contents,
            ContentComponent::Quote(kw) => Self::Quote(kw),
            ContentComponent::Leader(lt) => Self::Leader(lt),
            // cov:ignore: cross-crate `#[non_exhaustive]` catch-all — stable
            // Rust requires the `_` arm for exhaustive matching on
            // ContentComponent defined in raikiri-style; unreachable until
            // raikiri-style adds a variant this crate has not yet mirrored
            // (e.g. once raikiri-style lands `ContentComponent::Element`,
            // this arm's coverage window opens until a bridge arm is added).
            // Reporting `UnsupportedVariant` at runtime is the fail-closed
            // discipline for raikiri-style landing ahead of raikiri-traits —
            // see type-level docstring "Element variant" section.
            _ => return Err(ContentValueConvertError::UnsupportedVariant),
        })
    }
}

#[cfg(test)]
mod pagebox_px_baseline_tests {
    use super::*;

    #[test]
    fn a4_dimensions_match_css_px_conversion() {
        // 210mm × 297mm を CSS px (1/96 in) 換算:
        //   width  = 210mm × 96/25.4 ≈ 793.7008
        //   height = 297mm × 96/25.4 ≈ 1122.5197
        assert!(
            (PageBox::A4.width - 793.7008).abs() < 0.001,
            "A4.width should be ~793.7008 px, got {}",
            PageBox::A4.width
        );
        assert!(
            (PageBox::A4.height - 1122.5197).abs() < 0.001,
            "A4.height should be ~1122.5197 px, got {}",
            PageBox::A4.height
        );
    }

    #[test]
    fn us_letter_dimensions_match_exact_integers() {
        // 8.5in × 11in @ 96 DPI = 816 × 1056 px exactly
        assert_eq!(PageBox::US_LETTER.width, 816.0);
        assert_eq!(PageBox::US_LETTER.height, 1056.0);
    }

    #[test]
    fn pagebox_default_is_a4() {
        assert_eq!(PageBox::default(), PageBox::A4);
    }

    #[test]
    fn pagebox_from_page_size_resolves_lengths_and_orientation() {
        let box_size = PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
            width: raikiri_style::Length::Px(300.0),
            height: raikiri_style::Length::Px(50.0),
        }));
        assert_eq!(
            box_size,
            PageBox {
                width: 300.0,
                height: 50.0
            }
        );

        let landscape = PageBox::from_page_size(Some(raikiri_style::PageSize::Named {
            keyword: Some(raikiri_style::PageSizeKeyword::A5),
            orientation: Some(raikiri_style::PageOrientation::Landscape),
        }));
        assert!(landscape.width > landscape.height);
    }

    #[test]
    fn pagebox_from_page_size_covers_orientation_swaps() {
        assert_eq!(
            apply_page_orientation(50.0, 300.0, Some(PageOrientation::Landscape)),
            (300.0, 50.0)
        );
        assert_eq!(
            apply_page_orientation(300.0, 50.0, Some(PageOrientation::Portrait)),
            (50.0, 300.0)
        );
        assert_eq!(apply_page_orientation(300.0, 50.0, None), (300.0, 50.0));
    }

    #[test]
    fn pagebox_from_page_size_covers_length_units() {
        let cases = [
            (raikiri_style::Length::Px(1.0), 1.0),
            (raikiri_style::Length::Pt(1.0), 96.0 / 72.0),
            (raikiri_style::Length::Cm(1.0), 96.0 / 2.54),
            (raikiri_style::Length::Mm(1.0), 96.0 / 25.4),
            (raikiri_style::Length::Q(1.0), 96.0 / 101.6),
            (raikiri_style::Length::In(1.0), 96.0),
            (raikiri_style::Length::Pc(1.0), 16.0),
            (raikiri_style::Length::Em(1.0), 16.0),
            (raikiri_style::Length::Rem(1.0), 16.0),
            (raikiri_style::Length::Ex(1.0), 8.0),
            (raikiri_style::Length::Ch(1.0), 8.0),
            (raikiri_style::Length::Ic(1.0), 16.0),
            (raikiri_style::Length::Rex(1.0), 8.0),
            (raikiri_style::Length::Rch(1.0), 8.0),
            (raikiri_style::Length::Ric(1.0), 16.0),
            (raikiri_style::Length::Lh(1.0), 16.0),
            (raikiri_style::Length::Rlh(1.0), 16.0),
        ];
        for (length, expected) in cases {
            let page = PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
                width: length,
                height: raikiri_style::Length::Px(1.0),
            }));
            assert!((page.width - expected).abs() < 0.001, "got {}", page.width);
        }
    }

    #[test]
    fn pagebox_from_page_size_covers_named_keywords_and_invalid_fallbacks() {
        let keywords = [
            raikiri_style::PageSizeKeyword::A5,
            raikiri_style::PageSizeKeyword::A4,
            raikiri_style::PageSizeKeyword::A3,
            raikiri_style::PageSizeKeyword::B5,
            raikiri_style::PageSizeKeyword::B4,
            raikiri_style::PageSizeKeyword::JisB5,
            raikiri_style::PageSizeKeyword::JisB4,
            raikiri_style::PageSizeKeyword::Letter,
            raikiri_style::PageSizeKeyword::Legal,
            raikiri_style::PageSizeKeyword::Ledger,
        ];
        for keyword in keywords {
            let page = PageBox::from_page_size(Some(raikiri_style::PageSize::Named {
                keyword: Some(keyword),
                orientation: None,
            }));
            assert!(page.width > 0.0 && page.height > page.width);
        }

        assert_eq!(
            PageBox::from_page_size(Some(raikiri_style::PageSize::Named {
                keyword: None,
                orientation: None,
            })),
            PageBox::A4
        );
        assert_eq!(
            PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
                width: raikiri_style::Length::Px(10.0),
                height: raikiri_style::Length::Percent(50.0),
            })),
            PageBox::A4
        );
        assert_eq!(
            PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
                width: raikiri_style::Length::Px(0.0),
                height: raikiri_style::Length::Px(10.0),
            })),
            PageBox::A4
        );
    }

    #[test]
    fn pagebox_from_page_size_uses_a4_for_auto_or_invalid_relative_input() {
        assert_eq!(
            PageBox::from_page_size(Some(raikiri_style::PageSize::Auto)),
            PageBox::A4
        );
        assert_eq!(
            PageBox::from_page_size(Some(raikiri_style::PageSize::Lengths {
                width: raikiri_style::Length::Percent(50.0),
                height: raikiri_style::Length::Px(50.0),
            })),
            PageBox::A4
        );
    }
}

#[cfg(test)]
mod pagedefaults_tests {
    use super::*;

    #[test]
    fn pagedefaults_default_uses_a4() {
        let d = PageDefaults::default();
        assert_eq!(d.page_box, PageBox::A4);
    }

    #[test]
    fn pagedefaults_new_is_default() {
        assert_eq!(
            PageDefaults::new().page_box,
            PageDefaults::default().page_box
        );
    }

    #[test]
    fn pagedefaults_builder_sets_page_box() {
        let d = PageDefaults::builder().page_box(PageBox::US_LETTER).build();
        assert_eq!(d.page_box, PageBox::US_LETTER);
    }

    #[test]
    fn pagedefaults_builder_default_matches_pagedefaults_default() {
        let via_builder = PageDefaults::builder().build();
        let via_default = PageDefaults::default();
        assert_eq!(via_builder.page_box, via_default.page_box);
    }
}

#[cfg(test)]
mod gcpm_directive_populate_tests {
    //! GcpmDirective canonical 6 variant construction pins。
    //!
    //! design doc §7.1 line 1913-1920 verbatim shape。variant 追加 / rename /
    //! payload type 変更で fail、`#[non_exhaustive]` catch-all は無し
    //! (crate-local match は non_exhaustive の enforce 外)。

    use super::*;

    #[test]
    fn counter_increment_construct_and_payload_visible() {
        let d = GcpmDirective::CounterIncrement {
            name: Symbol::new("chapter"),
            delta: 1,
        };
        match d {
            GcpmDirective::CounterIncrement { name, delta } => {
                assert_eq!(name.as_str(), "chapter");
                assert_eq!(delta, 1);
            }
            other => panic!("expected CounterIncrement, got {other:?}"),
        }
    }

    #[test]
    fn counter_reset_construct_negative_value() {
        let d = GcpmDirective::CounterReset {
            name: Symbol::new("section"),
            value: -3,
        };
        match d {
            GcpmDirective::CounterReset { name, value } => {
                assert_eq!(name.as_str(), "section");
                assert_eq!(value, -3);
            }
            other => panic!("expected CounterReset, got {other:?}"),
        }
    }

    #[test]
    fn counter_set_construct_zero_value() {
        let d = GcpmDirective::CounterSet {
            name: Symbol::new("page"),
            value: 0,
        };
        match d {
            GcpmDirective::CounterSet { name, value } => {
                assert_eq!(name.as_str(), "page");
                assert_eq!(value, 0);
            }
            other => panic!("expected CounterSet, got {other:?}"),
        }
    }

    #[test]
    fn string_set_carries_content_source_items() {
        let source = ContentSource::new(vec![ContentValueItem::Literal(String::from("Ch. "))]);
        let d = GcpmDirective::StringSet {
            name: Symbol::new("heading"),
            source: source.clone(),
        };
        match d {
            GcpmDirective::StringSet { name, source: s } => {
                assert_eq!(name.as_str(), "heading");
                assert_eq!(s, source);
                assert_eq!(s.items.len(), 1);
            }
            other => panic!("expected StringSet, got {other:?}"),
        }
    }

    #[test]
    fn register_running_carries_template_id() {
        let template_id = RunningTemplateId::new(NodeId::new(42));
        let d = GcpmDirective::RegisterRunning {
            name: Symbol::new("header"),
            template_id,
        };
        match d {
            GcpmDirective::RegisterRunning {
                name,
                template_id: tid,
            } => {
                assert_eq!(name.as_str(), "header");
                assert_eq!(tid, template_id);
                assert_eq!(tid.0, NodeId::new(42));
            }
            other => panic!("expected RegisterRunning, got {other:?}"),
        }
    }

    #[test]
    fn register_target_carries_fragment_id() {
        let d = GcpmDirective::RegisterTarget {
            fragment_id: Symbol::new("intro"),
        };
        match d {
            GcpmDirective::RegisterTarget { fragment_id } => {
                assert_eq!(fragment_id.as_str(), "intro");
            }
            other => panic!("expected RegisterTarget, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod content_value_item_populate_tests {
    //! ContentValueItem canonical 10 variant construction pins と、
    //! Image/Contents/Quote/Leader 4 variant construction pins
    //! (design doc §7.1 canonical 10 の外、raikiri-style
    //! `ContentComponent` 1:1 mirror)。
    //!
    //! design doc §7.1 line 1926-1937 verbatim shape (canonical 10 分)。

    use super::*;

    #[test]
    fn literal_string_payload() {
        let c = ContentValueItem::Literal(String::from("hello"));
        match c {
            ContentValueItem::Literal(s) => {
                assert_eq!(s.as_str(), "hello");
            }
            other => panic!("expected Literal, got {other:?}"),
        }
    }

    #[test]
    fn counter_default_style_is_decimal() {
        let c = ContentValueItem::Counter {
            name: Symbol::new("chapter"),
            style: CounterStyle::default(),
        };
        match c {
            ContentValueItem::Counter { name, style } => {
                assert_eq!(name.as_str(), "chapter");
                assert_eq!(style, CounterStyle::Decimal);
            }
            other => panic!("expected Counter, got {other:?}"),
        }
    }

    #[test]
    fn counters_separator_payload() {
        let c = ContentValueItem::Counters {
            name: Symbol::new("section"),
            separator: String::from("."),
            style: CounterStyle::default(),
        };
        match c {
            ContentValueItem::Counters {
                name,
                separator,
                style,
            } => {
                assert_eq!(name.as_str(), "section");
                assert_eq!(separator, ".");
                assert_eq!(style, CounterStyle::Decimal);
            }
            other => panic!("expected Counters, got {other:?}"),
        }
    }

    #[test]
    fn string_default_fetch_mode_is_first() {
        let c = ContentValueItem::String {
            name: Symbol::new("running-title"),
            fetch: StringFetchMode::default(),
        };
        match c {
            ContentValueItem::String { name, fetch } => {
                assert_eq!(name.as_str(), "running-title");
                assert_eq!(fetch, StringFetchMode::First);
            }
            other => panic!("expected String, got {other:?}"),
        }
    }

    #[test]
    fn element_variant_construct() {
        let c = ContentValueItem::Element {
            name: Symbol::new("header"),
        };
        match c {
            ContentValueItem::Element { name } => {
                assert_eq!(name.as_str(), "header");
            }
            other => panic!("expected Element, got {other:?}"),
        }
    }

    #[test]
    fn content_default_part_is_content() {
        let c = ContentValueItem::Content {
            part: ContentPart::default(),
        };
        match c {
            ContentValueItem::Content { part } => {
                assert_eq!(part, ContentPart::Content);
            }
            other => panic!("expected Content, got {other:?}"),
        }
    }

    #[test]
    fn attr_variant_construct() {
        let c = ContentValueItem::Attr {
            name: Symbol::new("data-title"),
        };
        match c {
            ContentValueItem::Attr { name } => {
                assert_eq!(name.as_str(), "data-title");
            }
            other => panic!("expected Attr, got {other:?}"),
        }
    }

    #[test]
    fn target_counter_url_payload() {
        let url = Url::parse("https://example.com/#foo").expect("valid URL");
        let c = ContentValueItem::TargetCounter {
            url: url.clone(),
            name: Symbol::new("chapter"),
            style: CounterStyle::default(),
        };
        match c {
            ContentValueItem::TargetCounter {
                url: u,
                name,
                style,
            } => {
                assert_eq!(u, url);
                assert_eq!(name.as_str(), "chapter");
                assert_eq!(style, CounterStyle::Decimal);
            }
            other => panic!("expected TargetCounter, got {other:?}"),
        }
    }

    #[test]
    fn target_counters_sep_field_name_matches_design_doc() {
        // design doc §7.1 line 1935: `sep: String` (not `separator`)。
        // Regression check for the deliberate field-name deviation from
        // ContentComponent::TargetCounters (which uses `separator`).
        let url = Url::parse("https://example.com/#foo").expect("valid URL");
        let c = ContentValueItem::TargetCounters {
            url: url.clone(),
            name: Symbol::new("section"),
            sep: String::from("."),
            style: CounterStyle::default(),
        };
        match c {
            ContentValueItem::TargetCounters {
                url: u,
                name,
                sep,
                style,
            } => {
                assert_eq!(u, url);
                assert_eq!(name.as_str(), "section");
                assert_eq!(sep, ".");
                assert_eq!(style, CounterStyle::Decimal);
            }
            other => panic!("expected TargetCounters, got {other:?}"),
        }
    }

    #[test]
    fn target_text_default_part_is_content() {
        let url = Url::parse("https://example.com/#foo").expect("valid URL");
        let c = ContentValueItem::TargetText {
            url: url.clone(),
            part: ContentPart::default(),
        };
        match c {
            ContentValueItem::TargetText { url: u, part } => {
                assert_eq!(u, url);
                assert_eq!(part, ContentPart::Content);
            }
            other => panic!("expected TargetText, got {other:?}"),
        }
    }

    #[test]
    fn image_url_payload() {
        let url = Url::parse("https://example.com/logo.png").expect("valid URL");
        let c = ContentValueItem::Image { url: url.clone() };
        match c {
            ContentValueItem::Image { url: u } => {
                assert_eq!(u, url);
            }
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            other => panic!("expected Image, got {other:?}"),
        }
    }

    #[test]
    fn contents_unit_variant_construct() {
        let c = ContentValueItem::Contents;
        assert!(matches!(c, ContentValueItem::Contents));
    }

    #[test]
    fn quote_payload_covers_all_keywords() {
        for kw in [
            QuoteKeyword::OpenQuote,
            QuoteKeyword::CloseQuote,
            QuoteKeyword::NoOpenQuote,
            QuoteKeyword::NoCloseQuote,
        ] {
            let c = ContentValueItem::Quote(kw);
            match c {
                ContentValueItem::Quote(payload) => assert_eq!(payload, kw),
                // cov:ignore: panic-message literal only executed on
                // assertion failure, which doesn't happen while this test
                // passes.
                other => panic!("expected Quote, got {other:?}"),
            }
        }
    }

    #[test]
    fn leader_payload_covers_all_types() {
        for lt in [
            LeaderType::Dotted,
            LeaderType::Solid,
            LeaderType::Space,
            LeaderType::String(SmolStr::new("~")),
        ] {
            let c = ContentValueItem::Leader(lt.clone());
            match c {
                ContentValueItem::Leader(payload) => assert_eq!(payload, lt),
                // cov:ignore: panic-message literal only executed on
                // assertion failure, which doesn't happen while this test
                // passes.
                other => panic!("expected Leader, got {other:?}"),
            }
        }
    }
}

#[cfg(test)]
mod content_component_bridge_tests {
    //! [`TryFrom<ContentComponent> for ContentValueItem`] roundtrip pins
    //! (canonical taxonomy conversion; Image/Contents/Quote/Leader arms
    //! added as a 1:1 mirror of raikiri-style `ContentComponent`)。
    //!
    //! Coverage: 13 of 14 [`ContentValueItem`] variants — [`Element`] は
    //! [`ContentComponent::Element`] 未実装のため bridge 経路では現在到達
    //! 不能。variant 追加時に arm を extend する。

    use super::*;

    #[test]
    fn literal_bridge_converts_smol_str_to_string() {
        let cc = ContentComponent::Literal(SmolStr::new("hello"));
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Literal");
        assert_eq!(cvi, ContentValueItem::Literal(String::from("hello")));
    }

    #[test]
    fn counter_bridge_wraps_name_in_symbol() {
        let cc = ContentComponent::Counter {
            name: SmolStr::new("chapter"),
            style: CounterStyle::default(),
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Counter");
        assert_eq!(
            cvi,
            ContentValueItem::Counter {
                name: Symbol::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn counters_bridge_preserves_separator_field_name() {
        // ContentComponent::Counters は `separator`、ContentValueItem::Counters も
        // `separator` (design doc §7.1 line 1929 verbatim)。両者同名なので
        // straight mapping。
        let cc = ContentComponent::Counters {
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::default(),
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Counters");
        assert_eq!(
            cvi,
            ContentValueItem::Counters {
                name: Symbol::new("section"),
                separator: String::from("."),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn string_bridge_preserves_fetch_mode() {
        let cc = ContentComponent::String {
            name: SmolStr::new("title"),
            fetch: StringFetchMode::Last,
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for String");
        assert_eq!(
            cvi,
            ContentValueItem::String {
                name: Symbol::new("title"),
                fetch: StringFetchMode::Last,
            }
        );
    }

    #[test]
    fn attr_bridge_wraps_name_in_symbol() {
        let cc = ContentComponent::Attr {
            name: SmolStr::new("data-title"),
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Attr");
        assert_eq!(
            cvi,
            ContentValueItem::Attr {
                name: Symbol::new("data-title"),
            }
        );
    }

    #[test]
    fn target_counter_bridge_parses_url() {
        let cc = ContentComponent::TargetCounter {
            url: String::from("https://example.com/#foo"),
            name: SmolStr::new("chapter"),
            style: CounterStyle::default(),
        };
        let cvi = ContentValueItem::try_from(cc).expect("valid URL");
        let expected_url = Url::parse("https://example.com/#foo").expect("URL literal parses");
        assert_eq!(
            cvi,
            ContentValueItem::TargetCounter {
                url: expected_url,
                name: Symbol::new("chapter"),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn target_counters_bridge_maps_separator_to_sep() {
        // ContentComponent::TargetCounters は `separator`、design doc §7.1
        // line 1935 の ContentValueItem::TargetCounters は `sep` — bridge
        // で名称変換される。
        let cc = ContentComponent::TargetCounters {
            url: String::from("https://example.com/#foo"),
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::default(),
        };
        let cvi = ContentValueItem::try_from(cc).expect("valid URL");
        let expected_url = Url::parse("https://example.com/#foo").expect("URL literal parses");
        assert_eq!(
            cvi,
            ContentValueItem::TargetCounters {
                url: expected_url,
                name: Symbol::new("section"),
                sep: String::from("."),
                style: CounterStyle::Decimal,
            }
        );
    }

    #[test]
    fn target_text_bridge_parses_url() {
        let cc = ContentComponent::TargetText {
            url: String::from("https://example.com/#foo"),
            part: ContentPart::Before,
        };
        let cvi = ContentValueItem::try_from(cc).expect("valid URL");
        let expected_url = Url::parse("https://example.com/#foo").expect("URL literal parses");
        assert_eq!(
            cvi,
            ContentValueItem::TargetText {
                url: expected_url,
                part: ContentPart::Before,
            }
        );
    }

    #[test]
    fn content_bridge_maps_text_keyword_to_content_part() {
        // ContentTextKeyword::Text (GCPM 3 §1.1.1.1) →
        // ContentPart::Content (CSS Content 3 §2.6.3): 両者とも「要素の
        // string value 全体」を指す同一概念への canonical mapping
        // (spec の "default" 宣言には依らない — 詳細は
        // content_text_keyword_to_content_part の doc comment 参照)。
        let cc = ContentComponent::Content {
            keyword: ContentTextKeyword::Text,
        };
        let cvi = ContentValueItem::try_from(cc).expect("infallible for Content");
        assert_eq!(
            cvi,
            ContentValueItem::Content {
                part: ContentPart::Content,
            }
        );
    }

    #[test]
    fn content_bridge_maps_before_after_first_letter_verbatim() {
        // 非-default keyword は同名の ContentPart 値に mapping。
        for (kw, expected) in [
            (ContentTextKeyword::Before, ContentPart::Before),
            (ContentTextKeyword::After, ContentPart::After),
            (ContentTextKeyword::FirstLetter, ContentPart::FirstLetter),
        ] {
            let cvi = ContentValueItem::try_from(ContentComponent::Content { keyword: kw })
                .expect("infallible for Content");
            assert_eq!(
                cvi,
                ContentValueItem::Content { part: expected },
                "keyword {kw:?} should map to part {expected:?}"
            );
        }
    }

    #[test]
    fn image_bridge_parses_url() {
        let cc = ContentComponent::Image {
            url: String::from("https://example.com/logo.png"),
        };
        let cvi = ContentValueItem::try_from(cc).expect("valid URL");
        let expected_url = Url::parse("https://example.com/logo.png").expect("URL literal parses");
        assert_eq!(cvi, ContentValueItem::Image { url: expected_url });
    }

    #[test]
    fn contents_bridge_maps_to_unit_variant() {
        let cvi = ContentValueItem::try_from(ContentComponent::Contents)
            .expect("infallible for Contents");
        assert_eq!(cvi, ContentValueItem::Contents);
    }

    #[test]
    fn quote_bridge_passes_keyword_through() {
        for kw in [
            QuoteKeyword::OpenQuote,
            QuoteKeyword::CloseQuote,
            QuoteKeyword::NoOpenQuote,
            QuoteKeyword::NoCloseQuote,
        ] {
            let cvi = ContentValueItem::try_from(ContentComponent::Quote(kw))
                .expect("infallible for Quote");
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(cvi, ContentValueItem::Quote(kw), "keyword {kw:?} roundtrip");
        }
    }

    #[test]
    fn leader_bridge_passes_type_through() {
        for lt in [
            LeaderType::Dotted,
            LeaderType::Solid,
            LeaderType::Space,
            LeaderType::String(SmolStr::new("~")),
        ] {
            let cvi = ContentValueItem::try_from(ContentComponent::Leader(lt.clone()))
                .expect("infallible for Leader");
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                cvi,
                ContentValueItem::Leader(lt.clone()),
                "leader type {lt:?} roundtrip"
            );
        }
    }

    #[test]
    fn image_bridge_returns_invalid_url_error() {
        // Invalid URL (relative URL に base 無し) は InvalidUrl error
        // (sibling target_counter_bridge_returns_invalid_url_error と同じ pattern)。
        let cc = ContentComponent::Image {
            url: String::from("not a url"),
        };
        let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
        assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
    }

    #[test]
    fn target_counter_bridge_returns_invalid_url_error() {
        // Invalid URL (relative URL に base 無し) は InvalidUrl error。
        let cc = ContentComponent::TargetCounter {
            url: String::from("not a url"),
            name: SmolStr::new("chapter"),
            style: CounterStyle::default(),
        };
        let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
        assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
    }

    #[test]
    fn target_counters_bridge_propagates_url_parse_error() {
        let cc = ContentComponent::TargetCounters {
            url: String::from("not a url"),
            name: SmolStr::new("section"),
            separator: String::from("."),
            style: CounterStyle::default(),
        };
        let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
        assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
    }

    #[test]
    fn target_text_bridge_propagates_url_parse_error() {
        let cc = ContentComponent::TargetText {
            url: String::from("not a url"),
            part: ContentPart::default(),
        };
        let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
        assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
    }

    #[test]
    fn convert_error_display_and_source_chain() {
        use std::error::Error as _;
        // InvalidUrl は source() で url::ParseError を surface。
        let parse_err = Url::parse("not a url").expect_err("invalid URL");
        let err = ContentValueConvertError::InvalidUrl(parse_err);
        assert!(err.to_string().contains("invalid URL"));
        assert!(err.source().is_some());

        // UnsupportedVariant は source() none、Display で origin cite。
        let uv = ContentValueConvertError::UnsupportedVariant;
        assert!(uv.to_string().contains("unsupported ContentComponent"));
        assert!(uv.source().is_none());
    }
}

#[cfg(test)]
mod content_source_tests {
    use super::*;

    #[test]
    fn content_source_default_is_empty() {
        let cs = ContentSource::default();
        assert!(cs.items.is_empty());
    }

    #[test]
    fn content_source_new_wraps_vec() {
        let items = vec![
            ContentValueItem::Literal(String::from("Ch. ")),
            ContentValueItem::Counter {
                name: Symbol::new("chapter"),
                style: CounterStyle::default(),
            },
        ];
        let cs = ContentSource::new(items.clone());
        assert_eq!(cs.items, items);
    }

    #[test]
    fn content_source_struct_update_from_default() {
        // #[non_exhaustive] public struct の consumer construct pattern
        // (sibling PageBox の struct-update pattern 継承)。
        let cs = ContentSource {
            items: vec![ContentValueItem::Literal(String::from("hello"))],
            ..Default::default()
        };
        assert_eq!(cs.items.len(), 1);
    }
}

#[cfg(test)]
mod running_template_id_tests {
    use super::*;

    #[test]
    fn running_template_id_wraps_node_id() {
        let id = RunningTemplateId::new(NodeId::new(42));
        assert_eq!(id.0, NodeId::new(42));
    }

    #[test]
    fn running_template_id_copy_hash_eq_derives() {
        use std::collections::HashMap;
        let id1 = RunningTemplateId::new(NodeId::new(1));
        let id2 = id1; // Copy
        assert_eq!(id1, id2);
        // Hash + Eq — HashMap key として使える (raikiri-dom
        // RunningTemplateStore.parsed_templates keying rationale)。
        let mut m: HashMap<RunningTemplateId, &'static str> = HashMap::new();
        m.insert(id1, "template-1");
        assert_eq!(m.get(&id2), Some(&"template-1"));
    }

    #[test]
    fn running_template_id_ord_derives() {
        let id1 = RunningTemplateId::new(NodeId::new(1));
        let id2 = RunningTemplateId::new(NodeId::new(2));
        assert!(id1 < id2);
    }
}

#[cfg(test)]
mod page_fragment_tests {
    use super::*;

    #[test]
    fn fragment_item_split_and_repeat_semantics_are_distinct() {
        let split = PageFragmentItem::new(
            NodeId::new(7),
            PageFragmentRect::new(0.0, 0.0, 10.0, 5.0),
            PageFragmentKind::Box,
            0,
            2,
            false,
        );
        assert!(split.is_split());

        let repeat = PageFragmentItem::new(
            NodeId::new(7),
            PageFragmentRect::new(0.0, 0.0, 10.0, 5.0),
            PageFragmentKind::Box,
            0,
            2,
            true,
        );
        assert!(!repeat.is_split());
        assert!(!PageFragmentLineRange::new(0, 1).is_empty());
        assert!(PageFragmentLineRange::new(1, 1).is_empty());
    }
}
