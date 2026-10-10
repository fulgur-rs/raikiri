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

use crate::dom::{NodeId, Symbol};
use raikiri_style::property::{
    ContentComponent, ContentPart, ContentTextKeyword, CounterStyle, LeaderType, QuoteKeyword,
    StringFetchMode,
};
use raikiri_style::{Length, PageOrientation, PageSize, PageSizeKeyword};
#[cfg(test)]
use smol_str::SmolStr;
use url::Url;

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
            ContentComponent::Element { name, .. } => Self::Element {
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
mod tests;
