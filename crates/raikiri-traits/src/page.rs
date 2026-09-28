//! Page-related neutral model types.
//!
//! Types gathered here are governed by §5 (Pipeline), §7 (GCPM), §9 (PageBox),
//! and §11 (Paint). For now they are opaque placeholders; later tasks will
//! populate fields and methods incrementally.
//!
//! Input / constructible structs have `#[non_exhaustive]`, `impl Default`, and `pub fn new()`,
//! so external consumer crates can construct them with `X::new()` / `X::default()`
//! (consumers construct [`TargetInfo`] as input to `TargetRegistry::register`).
//! Output-only snapshot types (currently only [`PendingResolution`], populated
//! within the registry and returned through its API) are not constructed
//! directly by consumers, so Default / new are not required
//! (`#[non_exhaustive]` still applies to all public structs).

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

/// Resolved geometry metadata for one page in a page-fragment stream.
///
/// The values are page-local physical metadata: `content_box.x/y` is the
/// offset from the physical page origin, while a [`PageFragmentItem`]'s
/// rectangle is relative to that content-box origin. A producer supplies one
/// value per page when page size, margins, or insets vary; consumers must not
/// recompute these values from `page_name`.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PageFragmentPageGeometry {
    /// Zero-based page index to which this metadata belongs.
    pub page_index: u32,
    /// Resolved physical page box in CSS pixels.
    pub page_box: PageBox,
    /// Resolved page margins in CSS pixels.
    pub margins: PageFragmentInsets,
    /// Resolved border/padding inset inside the page margin area.
    pub content_insets: PageFragmentInsets,
    /// Page-local physical origin and flow dimensions in CSS pixels.
    ///
    /// `x/y` is the offset a consumer adds once to an item rectangle. The
    /// current producer keeps horizontal page insets out of the flow
    /// containing-block width, so `width` is the scheduled flow width while
    /// `x/y` still includes the resolved margin and inset.
    pub content_box: PageFragmentRect,
    /// Physical orientation derived from `page_box`.
    pub orientation: PageFragmentOrientation,
}

impl PageFragmentPageGeometry {
    /// Construct resolved metadata for one page.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        page_index: u32,
        page_box: PageBox,
        margins: PageFragmentInsets,
        content_insets: PageFragmentInsets,
        content_box: PageFragmentRect,
        orientation: PageFragmentOrientation,
    ) -> Self {
        Self {
            page_index,
            page_box,
            margins,
            content_insets,
            content_box,
            orientation,
        }
    }

    /// Return the same metadata for another page index.
    pub fn with_page_index(mut self, page_index: u32) -> Self {
        self.page_index = page_index;
        self
    }
}

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
    /// Page-local physical origin and flow dimensions in CSS pixels.
    ///
    /// `x/y` is the offset a consumer adds once to an item rectangle. The
    /// current producer keeps horizontal page insets out of the flow
    /// containing-block width, so `width` is the scheduled flow width while
    /// `x/y` still includes the resolved margin and inset.
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

    /// Construct a page snapshot from independently resolved page metadata.
    pub fn with_page_geometry(
        page_index: u32,
        geometry: PageFragmentPageGeometry,
        content_origin_y: f32,
        page_name: Option<String>,
    ) -> Self {
        Self {
            page_index,
            page_box: geometry.page_box,
            margins: geometry.margins,
            content_insets: geometry.content_insets,
            content_box: geometry.content_box,
            content_origin_y,
            page_name,
            orientation: geometry.orientation,
            items: Vec::new(),
        }
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
        Self::with_page_geometry(
            page_index,
            PageFragmentPageGeometry::new(
                page_index,
                page_box,
                margins,
                content_insets,
                content_box,
                orientation,
            ),
            content_origin_y,
            page_name,
        )
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
/// zero-based and `rect` is expressed in CSS pixels relative to the containing
/// page's `content_box` origin. To place it in the physical page coordinate
/// system, add `PageFragment::content_box.x/y` exactly once; do not add
/// `content_origin_y`, which is only the shared document-space slice selector.
/// The containing [`PageFragment`] carries the resolved page metadata; the
/// duplicated index keeps node-centric geometry usable without retaining the
/// page-vector wrapper.
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

/// Neutral link metadata attached to a page-local event.
///
/// The raw, trimmed `href` is preserved exactly as a string. URL resolution,
/// fragment lookup, and renderer-specific annotation construction remain
/// consumer responsibilities.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageFragmentLink {
    /// Source element's `href` attribute after surrounding whitespace is trimmed.
    pub href: String,
}

impl PageFragmentLink {
    /// Construct link metadata from an `href` value.
    pub fn new(href: impl Into<String>) -> Self {
        Self { href: href.into() }
    }
}

/// One page-local link event.
///
/// `anchor_node_id` identifies the owning `<a>` element while
/// `placement_node_id` identifies the exact [`PageFragmentItem`] whose
/// geometry is copied into this event. This distinction preserves link
/// identity when an anchor wraps text or replaced descendants.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PageFragmentLinkEvent {
    /// Stable source DOM node identifier of the owning anchor element.
    pub anchor_node_id: NodeId,
    /// Stable source DOM node identifier of the correlated placement.
    pub placement_node_id: NodeId,
    /// Zero-based page containing the placement.
    pub page_index: u32,
    /// Page-local CSS-pixel geometry copied from the placement.
    pub rect: PageFragmentRect,
    /// Fragment ordinal copied from the placement.
    pub fragment_index: u32,
    /// Total placements for this source node in the snapshot.
    pub fragment_count: u32,
    /// Whether the placement is a complete repeated copy.
    pub is_repeat: bool,
    /// Text line range copied from a text placement, if available.
    pub line_range: Option<PageFragmentLineRange>,
    /// Neutral link destination.
    pub link: PageFragmentLink,
}

impl PageFragmentLinkEvent {
    /// Construct a link event from a page placement.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        anchor_node_id: NodeId,
        placement_node_id: NodeId,
        page_index: u32,
        rect: PageFragmentRect,
        fragment_index: u32,
        fragment_count: u32,
        is_repeat: bool,
        line_range: Option<PageFragmentLineRange>,
        link: PageFragmentLink,
    ) -> Self {
        Self {
            anchor_node_id,
            placement_node_id,
            page_index,
            rect,
            fragment_index,
            fragment_count,
            is_repeat,
            line_range,
            link,
        }
    }
}

/// Neutral page-local consumer event vocabulary.
///
/// The link variant is deliberately the only initial event. It is sufficient
/// for a consumer to create its own link annotation while keeping PDF,
/// accessibility, and renderer-specific annotation values out of this crate.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PageFragmentEvent {
    /// A visible source link placement.
    Link(PageFragmentLinkEvent),
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
/// The `width` / `height` fields and `A4` / `US_LETTER` constants are implemented.
/// Page margins and margin-box declaration bags are carried by the page
/// cascade/scene layers rather than this two-dimensional paper-size value.
///
/// **Unit = CSS px** (1 CSS px = 1/96 inch in print contexts per CSS Values L4 §6.2
/// "Absolute Lengths" <https://www.w3.org/TR/css-values-4/#absolute-lengths>).
/// `from_page_size` converts cascaded CSS absolute units in `@page size`
/// into this CSS-px representation. Consumers constructing `PageBox`
/// directly must continue to pass CSS px.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PageBox {
    /// Page width (CSS px).
    pub width: f32,
    /// Page height (CSS px).
    pub height: f32,
    // Populate later:
    //   pub margins: Margins,
    //   pub margin_boxes: `[Option<MarginBox>; 16]`,
}

impl PageBox {
    /// A4 portrait: 210×297 mm = **793.70 × 1122.52 px** (@ 96 DPI anchor).
    /// CSS Paged Media Level 3 §7 default size.
    pub const A4: PageBox = PageBox {
        width: 793.7008,   // 210mm × 96/25.4
        height: 1122.5197, // 297mm × 96/25.4
    };

    /// US Letter portrait: 8.5×11 in = exactly **816 × 1056 px**.
    pub const US_LETTER: PageBox = PageBox {
        width: 816.0,
        height: 1056.0,
    };

    /// Construct a `PageBox` equal to `A4`. Retained as a stable
    /// zero-argument constructor even with `#[non_exhaustive]`.
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

/// Page-level defaults supplied by consumers when rendering begins. Currently
/// minimal, containing only paper size. Later add margins, orientation, named pages, etc.
///
/// All fields use CSS px (see `PageBox`). Consumers handle pt/mm/in conversion.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageDefaults {
    /// Default paper size (initial value when not overridden by `@page size`).
    /// Defaults to A4.
    pub page_box: PageBox,
}

impl PageDefaults {
    /// Shortcut equivalent to `Default`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a fluent builder.
    pub fn builder() -> PageDefaultsBuilder {
        PageDefaultsBuilder::default()
    }
}

/// Fluent builder for `PageDefaults`.
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct PageDefaultsBuilder {
    page_box: Option<PageBox>,
}

impl PageDefaultsBuilder {
    /// Set `page_box`.
    pub fn page_box(mut self, v: PageBox) -> Self {
        self.page_box = Some(v);
        self
    }

    /// Build; unset fields use their default values.
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
/// Neutral model for a buffer; raikiri-dom supplies the implementation (see §5).
/// Not yet implemented; to be populated later.
#[allow(missing_docs)]
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct LayoutBuffer {
    // Populate later.
}

impl LayoutBuffer {
    /// Construct an empty LayoutBuffer (placeholder, not yet populated).
    pub fn new() -> Self {
        Self::default()
    }
}

// `TargetRegistry` — runtime registry for emitting and resolving target-* placeholders.
// Merged the canonical shape from design doc §7.2 with raikiri-dom's
// `pub(crate)` shadow implementation. The canonical implementation lives
// in the sibling `target` submodule; this module only re-exports it.

/// RunningTemplate — registration of a `position: running(name)` template.
/// Not yet implemented; to be populated later.
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct RunningTemplate {
    // Populate later.
}

impl RunningTemplate {
    /// Construct an empty RunningTemplate (placeholder, not yet populated).
    pub fn new() -> Self {
        Self::default()
    }
}

/// FormData — neutral model for an application/x-www-form-urlencoded body.
/// Used by consumer network implementations (Body::Form(FormData)).
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct FormData {
    // Populate later:
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
/// design doc §7.0 line 1903-1905 "shared types → raikiri-traits" scope.
/// Referenced by a field of [`GcpmDirective::RegisterRunning`] (§7.1 line 1918).
///
/// Wraps a subtree-root [`NodeId`]. The root of an element with
/// `position: running(name)` is unique per element in the arena, so it
/// provides a stable per-template key (matching the keying rationale
/// for raikiri-dom's `RunningTemplateStore`). The newtype avoids mixing
/// `NodeId` values used for other purposes.
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
/// design doc §7.1 line 1917 `StringSet { name: Symbol, source: ContentSource }`.
/// Source representation after raikiri-style cascade resolves a
/// `string-set: name <content-list>` declaration and before raikiri-dom's
/// Phase B walk copies it into [`PageContext`] named-string state in the
/// four-snapshot mode of §2.7.2 (start / first / last / first-except).
///
/// A future consumer (raikiri-style bridge) will construct it from
/// [`Vec<ContentValueItem>`]. The public `items` field also supports
/// construction with `..Default::default()` (as for sibling [`PageBox`]).
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
/// `position: running(name)`), consumed by raikiri-dom Phase B
/// walk to update [`PageContext`] and [`TargetRegistry`] (design doc §7.1
/// lines 1907-1920 — "Producing directive", emitted by raikiri-style and applied by raikiri-dom).
///
/// **Producing side** of §7 GCPM: Phase B walk mutates the running counter tree,
/// four named-string snapshots, running bindings, and TargetRegistry.
///
/// **`#[non_exhaustive]` semantics** — same forward-compatibility contract
/// as sibling [`ContentValueItem`]: enum-level `#[non_exhaustive]` requires
/// a `_ =>` arm in downstream `match` but does not block construction
/// of existing tuple/struct variants. Changing the payload **type**
/// of an existing variant will break downstream constructors at compile time.
///
/// Replaces the earlier uninhabited placeholder with the six canonical
/// variants from design doc §7.1 lines 1913-1920.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcpmDirective {
    /// `counter-increment: name delta` — increment the specified counter `name` by `delta`
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#propdef-counter-increment>).
    CounterIncrement {
        /// Counter name (custom-ident).
        name: Symbol,
        /// Increment amount (spec default 1; negative for `counter-increment: name -3`).
        delta: i32,
    },
    /// `counter-reset: name value` — reset the specified counter `name` to `value`
    /// (CSS Lists 3 §4.1 <https://www.w3.org/TR/css-lists-3/#propdef-counter-reset>).
    CounterReset {
        /// Counter name (custom-ident).
        name: Symbol,
        /// Reset value (spec default 0).
        value: i32,
    },
    /// `counter-set: name value` — set counter `name` to `value` on this element
    /// (CSS Lists 3 §4.2 <https://www.w3.org/TR/css-lists-3/#propdef-counter-set>).
    CounterSet {
        /// Counter name (custom-ident).
        name: Symbol,
        /// Set value (spec default 0).
        value: i32,
    },
    /// `string-set: name <content-list>` — snapshot the resolved content-list
    /// into the four-snapshot named-string state of `name`
    /// (start / first / last / first-except; CSS GCPM 3 §1.1.1 <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>).
    StringSet {
        /// Named-string identifier.
        name: Symbol,
        /// Resolved source content-list ([`ContentSource`]).
        source: ContentSource,
    },
    /// `position: running(name)` — register the current subtree as a running template
    /// under `name` and store `template_id` (subtree root wrapped in
    /// [`RunningTemplateId`]) as the key in the [`RunningTemplate`] pool
    /// (CSS GCPM 3 §1.2.1 <https://www.w3.org/TR/css-gcpm-3/#running-syntax>).
    RegisterRunning {
        /// Running-template name (custom-ident).
        name: Symbol,
        /// Template identifier (subtree root wrapped).
        template_id: RunningTemplateId,
    },
    /// Register an in-document fragment ID in [`TargetRegistry`]
    /// (for later resolution by `target-counter()` / `target-counters()` /
    /// `target-text()`; design doc §7.2 TargetRegistry).
    RegisterTarget {
        /// Fragment ID (usually a Symbol view of the element's `id` attribute value).
        fragment_id: Symbol,
    },
}

/// Item from the resolved `content` property — the "consuming" counterpart
/// of [`GcpmDirective`] (design doc §7.1 lines 1923-1937:
/// raikiri-style cascade emits it, raikiri-dom resolves it to a concrete
/// string at paint time).
///
/// **`#[non_exhaustive]` semantics** — same forward-compatibility contract
/// as sibling [`GcpmDirective`] and [`raikiri_style::property::ContentComponent`].
///
/// **`TargetCounters::sep`** — design doc §7.1 line 1935 names the field `sep`.
///   [`raikiri_style::property::ContentComponent::TargetCounters::separator`]
/// is `separator`; use `sep` verbatim from the design doc. The [`TryFrom`]
/// implementation maps `separator → sep`.
///
/// # `Element` variant
///
/// `ContentValueItem::Element` is the design doc §7.1 line 1931
/// canonical representation of `element(name)`. The style parser and
/// bridge retain this name so the paint-side running-template resolver
/// can select the matching `position: running(name)` element.
///
/// Replaces the uninhabited placeholder with the 10 canonical variants
/// from design doc §7.1 lines 1926-1937.
///
/// [`Image`](Self::Image) / [`Contents`](Self::Contents) / [`Quote`](Self::Quote) /
/// [`Leader`](Self::Leader) are **not** among the 10 canonical variants
/// in design doc §7.1. They mirror four variants later
/// added to raikiri-style `ContentComponent` (CSS Content 3
/// §2.2/§2.3/§2.4.2/§2.5.1). [`QuoteKeyword`] / [`LeaderType`] reuse
/// raikiri-style types directly, following sibling [`Counter`](Self::Counter)
/// with its direct reuse of `style: CounterStyle`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentValueItem {
    /// bare `<string>` literal (`content: "hello"`).
    Literal(String),
    /// `counter(name, style?)` (CSS Lists 3 §4.7
    /// <https://www.w3.org/TR/css-lists-3/#counter-functions>).
    Counter {
        /// Counter name (custom-ident).
        name: Symbol,
        /// Counter style (spec default `decimal`).
        style: CounterStyle,
    },
    /// `counters(name, separator, style?)` (CSS Lists 3 §4.7).
    Counters {
        /// Counter name (custom-ident).
        name: Symbol,
        /// Separator string (nested counter stack join).
        separator: String,
        /// Counter style (spec default `decimal`).
        style: CounterStyle,
    },
    /// `string(name, mode?)` (CSS Content 3 §2.7.2
    /// <https://www.w3.org/TR/css-content-3/#string-function>).
    String {
        /// Named-string identifier.
        name: Symbol,
        /// Fetch mode (spec default `first`).
        fetch: StringFetchMode,
    },
    /// `element(name)` (CSS GCPM 3 §1.2.2
    /// <https://www.w3.org/TR/css-gcpm-3/#element-syntax>). Runtime resolution
    /// looks up raikiri-dom's [`RunningTemplate`] pool.
    ///
    /// **From-impl gap**: `ContentComponent::Element` is not implemented.
    /// See type-level docstring "Element variant" section.
    Element {
        /// Running-template name (custom-ident).
        name: Symbol,
    },
    /// `content(part?)` (CSS GCPM 3 §1.1.1.1
    /// <https://www.w3.org/TR/css-gcpm-3/#funcdef-content>) (`?` is accepted
    /// by raikiri but does not appear in the spec grammar). If the keyword
    /// is omitted, use [`ContentPart::Content`] as a fallback (not based on a
    /// spec statement that it is the "default"; see the docs for
    /// [`ContentTextKeyword`] for details).
    Content {
        /// Which part of the element's string value to insert.
        part: ContentPart,
    },
    /// `attr(name)` (CSS Content 3 §2.1
    /// <https://www.w3.org/TR/css-content-3/#strings>).
    Attr {
        /// Attribute name (null-namespace, CSS Content 3 §2.1).
        name: Symbol,
    },
    /// `target-counter(url, name, style?)` (CSS Content 3 §2.6.1
    /// <https://www.w3.org/TR/css-content-3/#target-counter>).
    TargetCounter {
        /// Target URL (parsed by [`Url::parse`] in the bridge).
        url: Url,
        /// Counter name (custom-ident).
        name: Symbol,
        /// Counter style (spec default `decimal`).
        style: CounterStyle,
    },
    /// `target-counters(url, name, separator, style?)` (CSS Content 3 §2.6.2
    /// <https://www.w3.org/TR/css-content-3/#target-counters>).
    ///
    /// Field name `sep` follows design doc §7.1 line 1935 verbatim.
    TargetCounters {
        /// Target URL (parsed by [`Url::parse`] in the bridge).
        url: Url,
        /// Counter name (custom-ident).
        name: Symbol,
        /// Separator string (design doc verbatim `sep`).
        sep: String,
        /// Counter style (spec default `decimal`).
        style: CounterStyle,
    },
    /// `target-text(url, part?)` (CSS Content 3 §2.6.3
    /// <https://www.w3.org/TR/css-content-3/#target-text>).
    TargetText {
        /// Target URL (parsed by [`Url::parse`] in the bridge).
        url: Url,
        /// Which part to insert (`content` is the keyword for the entire
        /// string value of the element).
        part: ContentPart,
    },
    /// `<image>` (`url()` alternative) — CSS Content 3 §2.2
    /// <https://www.w3.org/TR/css-content-3/#content-uri>.
    /// One-to-one mirror of [`ContentComponent::Image`].
    ///
    /// **`<content-replacement>` is not implemented**:
    /// The same caveat from the docs for
    /// [`raikiri_style::property::ContentComponent::Image`] applies here:
    /// treating a `content-list` with a single `Image` item as
    /// `<content-replacement>` (suppressing the pseudo-element and replacing
    /// the whole element) is not implemented in raikiri-dom Phase B / paint time.
    /// Its shape (a single `Image` in `Vec<ContentValueItem>`) is enough for downstream reconstruction.
    Image {
        /// Image URL (parsed by [`Url::parse`] in the bridge, matching sibling
        /// [`TargetCounter`](Self::TargetCounter)).
        url: Url,
    },
    /// `contents` keyword — CSS Content 3 §2.3
    /// <https://www.w3.org/TR/css-content-3/#element-content>.
    /// One-to-one mirror of [`ContentComponent::Contents`].
    Contents,
    /// `<quote>` (`open-quote` / `close-quote` / `no-open-quote` /
    /// `no-close-quote`) — CSS Content 3 §2.4.2
    /// <https://www.w3.org/TR/css-content-3/#quote-values>.
    /// One-to-one mirror of [`ContentComponent::Quote`];
    /// the payload directly reuses [`raikiri_style::property::QuoteKeyword`]
    /// (no duplicate type in traits).
    Quote(QuoteKeyword),
    /// `leader(<leader-type>)` — CSS Content 3 §2.5.1
    /// <https://www.w3.org/TR/css-content-3/#leader-function>.
    /// One-to-one mirror of [`ContentComponent::Leader`];
    /// the payload directly reuses [`raikiri_style::property::LeaderType`]
    /// (no duplicate type in traits).
    Leader(LeaderType),
}

/// Failure taxonomy for [`TryFrom<ContentComponent> for ContentValueItem`].
///
/// raikiri-style is a leaf crate without a `url` dependency, so target-*
/// variants store raw URLs as [`String`], parsed by [`Url::parse`] in the bridge.
/// Distinguish parsing failures from detection of a new variant added to
/// raikiri-style for forward compatibility.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentValueConvertError {
    /// Failed to parse the URL string of a target-* variant with [`Url::parse`].
    InvalidUrl(url::ParseError),
    /// raikiri-style [`ContentComponent`] gained a variant unsupported by this crate
    /// (the catch-all for cross-crate `#[non_exhaustive]`).
    ///
    /// Stable Rust cannot enforce an exhaustive match across crates for
    /// `#[non_exhaustive]` at compile time, so this is a runtime error. Extend
    /// this crate's [`TryFrom`] arm when raikiri-style adds a variant.
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

/// Mapping [`ContentTextKeyword`] → [`ContentPart`] (bridging the keyword in
/// `content(keyword)`). The spec spellings differ: GCPM 3 §1.1.1.1
/// `content()` and CSS Content 3 §2.6.3 `target-text()` use different
/// keywords for the same idea (`text` vs `content`). Map them by meaning
/// when consolidating them into [`ContentPart`] values, not by an alleged
/// spec "default": CSS Content 3 §2.6.3 does not specify the value when
/// the second argument of `target-text()` is omitted. GCPM 3 §1.1.1.1
/// calls `text` "the default value", but its grammar omits `?`
/// (the second argument is syntactically required), and how to define
/// "default" remains an open WG issue, not stable evidence in the TR.
///
/// - [`ContentTextKeyword::Text`] → [`ContentPart::Content`] (both denote
///   the element's entire string value)
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
    /// Convert to the canonical taxonomy.
    ///
    /// Convert 13 variants (`Element` is not yet implemented in raikiri-style;
    /// add an arm when it becomes available):
    ///
    /// - [`ContentComponent::Literal`] → [`ContentValueItem::Literal`] (convert to `String`)
    /// - [`ContentComponent::Counter`] → [`ContentValueItem::Counter`]
    ///   (`name: SmolStr` → [`Symbol::new`])
    /// - [`ContentComponent::Counters`] → [`ContentValueItem::Counters`]
    /// - [`ContentComponent::String`] → [`ContentValueItem::String`]
    /// - [`ContentComponent::Attr`] → [`ContentValueItem::Attr`]
    /// - [`ContentComponent::TargetCounter`] → [`ContentValueItem::TargetCounter`]
    ///   (parse URL with [`Url::parse`]; on failure return [`ContentValueConvertError::InvalidUrl`])
    /// - [`ContentComponent::TargetCounters`] → [`ContentValueItem::TargetCounters`]
    ///   (map field name `separator` → `sep`; design doc §7.1 line 1935 verbatim)
    /// - [`ContentComponent::TargetText`] → [`ContentValueItem::TargetText`]
    /// - [`ContentComponent::Content`] → [`ContentValueItem::Content`]
    ///   ([`ContentTextKeyword`] → [`ContentPart`] mapping)
    /// - [`ContentComponent::Image`] → [`ContentValueItem::Image`]
    ///   (parse URL with [`Url::parse`]; on failure return [`ContentValueConvertError::InvalidUrl`])
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
        // Convert 210mm × 297mm to CSS px (1/96 in):
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
    //! GcpmDirective canonical 6 variant construction pins.
    //!
    //! Verbatim shape from design doc §7.1 lines 1913-1920; adding / renaming
    //! variants or changing payload types fails; no `#[non_exhaustive]` catch-all
    //! (crate-local matches are not subject to non_exhaustive enforcement).

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
    //! Pin construction of the 10 canonical ContentValueItem variants and
    //! construction of the four Image/Contents/Quote/Leader variants
    //! (the latter four are outside the canonical 10 from design doc §7.1
    //! and mirror raikiri-style `ContentComponent` one-to-one).
    //!
    //! Verbatim shape for the canonical 10 from design doc §7.1 lines 1926-1937.

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
        // design doc §7.1 line 1935: `sep: String` (not `separator`).
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
    //! added as a 1:1 mirror of raikiri-style `ContentComponent`).
    //!
    //! Coverage: 13 of 14 [`ContentValueItem`] variants — [`Element`] is
    //! unreachable through the bridge because [`ContentComponent::Element`]
    //! is not yet implemented. Extend an arm when a variant is added.

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
        // Both ContentComponent::Counters and ContentValueItem::Counters have
        // a `separator` field (design doc §7.1 line 1929 verbatim), so the
        // mapping is direct.
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
        // ContentComponent::TargetCounters uses `separator`; design doc §7.1
        // line 1935 names ContentValueItem::TargetCounters' field `sep`, so the
        // bridge converts the field name.
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
        // Canonical mapping to ContentPart::Content (CSS Content 3 §2.6.3):
        // both denote the element's entire string value
        // (not based on a spec "default" statement; see the docs for
        // content_text_keyword_to_content_part).
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
        // Map non-default keywords to ContentPart values with the same names.
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
        // A relative URL without a base is an InvalidUrl error
        // (same pattern as sibling target_counter_bridge_returns_invalid_url_error).
        let cc = ContentComponent::Image {
            url: String::from("not a url"),
        };
        let err = ContentValueItem::try_from(cc).expect_err("relative URL fails without base");
        assert!(matches!(err, ContentValueConvertError::InvalidUrl(_)));
    }

    #[test]
    fn target_counter_bridge_returns_invalid_url_error() {
        // A relative URL without a base is an InvalidUrl error.
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
        // InvalidUrl exposes url::ParseError through source().
        let parse_err = Url::parse("not a url").expect_err("invalid URL");
        let err = ContentValueConvertError::InvalidUrl(parse_err);
        assert!(err.to_string().contains("invalid URL"));
        assert!(err.source().is_some());

        // UnsupportedVariant has no source(); Display cites its origin.
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
        // Consumer construction pattern for a #[non_exhaustive] public struct
        // (following sibling PageBox's struct-update pattern).
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
        // Hash + Eq: usable as a HashMap key (raikiri-dom
        // RunningTemplateStore.parsed_templates keying rationale).
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
    fn page_geometry_builders_preserve_metadata_and_page_index() {
        let mut page_box = PageBox::new();
        page_box.width = 100.0;
        page_box.height = 120.0;
        let margins = PageFragmentInsets::new(10.0, 11.0, 12.0, 13.0);
        let insets = PageFragmentInsets::new(2.0, 3.0, 4.0, 5.0);
        let content_box = PageFragmentRect::new(18.0, 12.0, 74.0, 94.0);
        let geometry = PageFragmentPageGeometry::new(
            0,
            page_box,
            margins,
            insets,
            content_box,
            PageFragmentOrientation::Portrait,
        );
        let shifted = geometry.with_page_index(3);
        assert_eq!(shifted.page_index, 3);
        assert_eq!(shifted.page_box, page_box);
        assert_eq!(shifted.content_box, content_box);

        let page = PageFragment::with_metadata(
            3,
            page_box,
            margins,
            insets,
            content_box,
            240.0,
            Some(String::from("named")),
            PageFragmentOrientation::Portrait,
        );
        assert_eq!(page.page_index, 3);
        assert_eq!(page.page_box, page_box);
        assert_eq!(page.margins, margins);
        assert_eq!(page.content_insets, insets);
        assert_eq!(page.content_box, content_box);
        assert_eq!(page.content_origin_y, 240.0);
        assert_eq!(page.page_name.as_deref(), Some("named"));
    }

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
