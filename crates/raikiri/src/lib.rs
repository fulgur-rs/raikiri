//! raikiri — umbrella facade, re-exports, and internal dogfooding APIs.
//!
//! Consumers can depend on the single `raikiri` crate for HTML parsing and
//! cascaded ComputedValues. This facade re-exports the necessary types,
//! traits, and functions from sub-crates. Document parsing, cascade
//! orchestration, and [`crate::layout`] returning an owned [`crate::DocumentLayout`]
//! originate in `raikiri-html`;
//! this crate only re-exports their entry points. It also supplies PageScene
//! and PageDrawables for internal dogfooding and validation, while the main
//! contract for external page-output consumers lives in `raikiri-html` and
//! `raikiri-dom`.
//!
//! Text is laid out by the shodo inline engine, with the installed fonts in
//! [`html_to_png()`] and [`RenderResources`], or with a font set built by
//! [`FontCollectionBuilder`] and passed to [`RenderResources::fonts`] or
//! [`html_to_png_with_render_fonts`].
//!
//! # Example
//!
//! ```
//! use raikiri::{build_cascaded, parse, ParseOptions};
//!
//! let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
//! let doc = parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
//! let result = build_cascaded(&doc).expect("the cascade succeeds");
//! assert!(!result.computed.is_empty(), "cascade populates per-node ComputedValues");
//! ```

#![allow(rustdoc::private_intra_doc_links)]
// cov:ignore: public re-export declarations have no executable body
pub use raikiri_html::{
    Anchor, AnchorIndex, DEFAULT_MAX_AGGREGATE_RESOURCE_BYTES, DEFAULT_MAX_RESOURCE_BYTES,
    DocumentLayout, DomView, Fragment, FragmentKind, HtmlDocument, LayoutOptions, LayoutStatus,
    Link, Page, PageGeometry, PageMode, RenderResources, RepeatKind, ResourceLimits,
    build_cascaded, build_cascaded_for_page, build_cascaded_with_consumer_properties,
    build_cascaded_with_media_context, build_cascaded_with_media_context_for_page,
    build_cascaded_with_media_context_for_page_and_consumer_properties,
    build_cascaded_with_options, build_rule_tree, build_rule_tree_with_consumer_properties,
    build_rule_tree_with_limits, layout, parse_html, parse_html_with_limits,
    parse_html_with_resources,
};

pub use raikiri_html::{
    BundledFont, FontCollection, FontCollectionBuildError, FontCollectionBuilder,
    MAX_BUNDLED_FONT_BYTES, RenderFonts,
};

mod html_to_png;
pub use html_to_png::{html_to_png, html_to_png_with_render_fonts, html_to_png_with_resolver};

// ── PageScene + PageDrawables dogfooding surface ───────
// Implementation is growing incrementally from placeholder structs (empty
// structs plus Default). These public types serve raikiri's internal
// dogfooding/validation, not Fulgur's page-output contract, which centers
// on raikiri-dom. Reusing the re-exported `raikiri_traits::NodeId` gives
// PageScene and Document the same node identity.
mod page_scene;
pub use page_scene::{
    Fragment as PageSceneFragment, Orientation, PageMetadata, PageScene, Pt, build_page_scene,
    build_page_scene_for_page, build_page_scene_for_page_named,
};

mod raster_budget;
// cov:ignore: public re-export declarations have no executable body
pub use raster_budget::{
    MAX_DOCUMENT_RASTER_BYTES, MAX_PAGE_RASTER_BYTES, MAX_RASTER_EDGE, RasterBufferBudget,
    RasterBufferSize,
};

mod page_drawables;
pub use page_drawables::{PageDrawables, TrackedMap};

mod entries;
pub use entries::{
    BlockEntry, BookmarkAnchorEntry, ImageEntry, LinkSpanEntry, ListItemEntry, MulticolRuleEntry,
    ParagraphEntry, SemanticEntry, SvgEntry, TableEntry, TransformEntry,
};

// ── Page geometry types ───────────────────────────────────────────────────
pub use raikiri_dom::{PageMargins, PageSlice, first_page_name};

// ── raikiri-traits: shared vocabulary + DOM traits + error taxonomy ────
// A consumer implementing `NetworkProvider` needs Request, FetchedResource,
// NetworkError, Method, Body, HeaderMap, AbortSignal, AbortController, and
// ResourceKind in the fetch signature. Re-export them together to avoid
// direct sub-crate dependencies.
#[rustfmt::skip]
pub use raikiri_traits::{
    // ── existing ──
    AbortController, AbortSignal, Body, CascadeError, ConsumerPropertyEvent,
    ConsumerPropertyObserver, ConsumerPropertyValue, DecodedImage, Dom, Element, FetchOutcome,
    FetchedResource, HeaderMap, ImageIntrinsicSize, ImagePixelSource, ImageRasterSize, Method,
    NetworkError, NetworkProvider,
    Node, NodeId, NodeKind, ParseError, QuirksMode, RenderError, RenderWarning,
    Request, ResourceKind, StylesheetKind, WarningKind,

    // ── errors / status ──
    RenderSummary, LimitKind, UnresolvedTarget, UnresolvedReason,
    EmittedSlotInfo, TargetSlotId, TargetKind, TargetDiscrepancy, ExhaustionPolicy,

    // ── plan mode ──
    DocumentPlan, PageSummary, BreakReason, TargetDefinition,

    // ── config ──
    RenderLimits, RenderLimitsBuilder,
    LookaheadConfig, LookaheadConfigBuilder,
    LayoutConfig, LayoutConfigBuilder,
    BatchConfig, BatchConfigBuilder,

    // ── paged model ──
    PageBox, PageContext, PageDefaults, PageDefaultsBuilder,
    LayoutBuffer, TargetRegistry, RunningTemplate, FormData,
    GcpmDirective, ContentValueItem,

    // ── neutral paint payload ──
    PaintClip,
    PaintColor, PaintInsets, PaintRect,
    // ── traits implemented by consumers ──
    ReplacedResolver, ResourcePolicy,

    // ── strategy traits ──
    LookaheadPolicy, TargetResolver, ReflowPolicy,
    ReflowAction, ContainerOverflowFallback, DirtyDeadline,
    ProbeContext, TargetRequest, ResolvedTarget,

    // ── resolver helpers ──
    IntrinsicBox, ResolvedIntrinsic, ResolveDisposition,
    ResolverRequest, ResolverError,

    // ── policy helpers ──
    PolicyViolation, ViolationType,

    // ── layout helpers ──
    LayoutError,

    // ── symbol ──
    Symbol,
};

// ── raikiri-html: parse pipeline entry ─────────────────────────────────
pub use raikiri_html::{MINIMAL_UA_CSS, ParseOptions, UncascadedDocument, parse};
pub use raikiri_html::{StylesheetPart, StylesheetSource};

// ── raikiri-dom: Document (the type of raikiri-html::UncascadedDocument.dom) ──
// Consumers need this to name `&raikiri::Document` explicitly.
pub use raikiri_dom::Document;

// ── raikiri-style: cascade pipeline output and value types ─────────────
// Consumers name the types of ComputedValues fields to read and write them,
// so the value types are also re-exported.
//
// After ComputedValues' length fields moved to **computed-value types**, five
// Computed* types were added. They are computed-layer leaf types (except
// ComputedBorder, a width/style/color struct rather than a scalar):
//
// - `ComputedLength`                 — `font_size` / `ComputedBorder::width()`
// - `ComputedLengthPercentage`       — per-side leaf of `padding`
// - `ComputedLengthPercentageOrAuto` — per-side leaf of `margin`, `width`, `height`
// - `ComputedLineHeight`             — `line_height`
// - `ComputedBorder`                 — per-side leaf of `border`
//
// **Correction:** These five are only leaves. The actual `padding`, `margin`,
// and `border` field types are four-side containers such as
// `Sides<ComputedLengthPercentage>`. When the leaves were first re-exported,
// `Sides<T>` was deliberately kept outside the approved surface. Thus those
// three fields still could not be named with explicit types. A later change
// re-exported `Sides` and closed that gap (see below).
//
// Keep `Length` (associated with the specified layer). It is no longer a
// ComputedValues field type, but the also-exported `PropertyValue` still
// carries `Length` in variant payloads such as
// `PropertyValue::FontSize(Length)`. Consumers reading declarations need to
// name it; removing it would strand users of `PropertyValue`.
//
// A `Length` payload does **not** imply a specified-layer value. The layer
// depends on where the `PropertyValue` came from. See the canonical rule in
// the `raikiri_style::Length` docs, which consumers can reach through this
// re-export. Specifically:
//
// - `RuleTree` / `Declaration` values (cascade inputs just after parsing)
//   are specified-layer values and may retain `Em`, `Rem`, `Pt`, or `Percent`.
// - `PageCascadeResult.declarations` from `raikiri_style::cascade_page` are
//   **computed-layer** values carried in `Length`. For exact guarantees and
//   exceptions, consult the canonical docs on
//   `raikiri_style::page::PageCascadeResult::declarations`; do not repeat
//   them here, as a previous copy drifted from the implementation. The
//   umbrella crate does not re-export `cascade_page`, so only consumers
//   depending directly on raikiri-style can access this path.
//
// `Sides<T>`, `LengthOrAuto`, `LineHeight`, and `Border` were added later.
// **This supersedes the earlier decision not to export Sides.** Their roles:
//
// - `Sides<T>` is a layer-agnostic top/right/bottom/left container. It is
//   parameterized for both specified values (e.g.
//   `PropertyValue::Padding(Sides<Length>)`) and computed values (e.g.
//   `ComputedValues.padding: Sides<ComputedLengthPercentage>`). Unlike the
//   other three additions, it is not itself a specified-layer type. It closes
//   the missing-container gap described above.
// - `LengthOrAuto` is specified-layer data, e.g. the payload of
//   `PropertyValue::MarginTop(LengthOrAuto)`. Its computed counterpart is
//   `ComputedLengthPercentageOrAuto`.
// - `LineHeight` is specified-layer data, e.g. the payload of
//   `PropertyValue::LineHeight(LineHeight)`. Its computed counterpart is
//   `ComputedLineHeight`.
// - `Border` is specified-layer data in
//   `PropertyValue::Border(Sides<Border>)`; its computed counterpart is
//   `ComputedBorder`. It is public through
//   `raikiri_style::property::Border`, not the raikiri_style crate root, so
//   it is imported separately below. `LineHeight` has the same constraint.
//
// **Further correction:** The Sides/Border addition left two gaps. First,
// `Border` is a `#[non_exhaustive]` struct, so a struct literal from raikiri
// fails (E0639), and merely exporting the type offered no public way to
// obtain a value. `Sides::all` can duplicate an existing Border across four
// sides but cannot create that initial value. Second, `BorderStyle` and
// `BorderColor`, the types of the other two fields besides `Border::width`,
// were not re-exported, so consumers could not read them with explicit types.
//
// Both gaps were addressed:
//
// - `raikiri_style::property::Border` gained `pub fn new() -> Self` (a thin
//   wrapper around `Self::default()`) and `impl Default for Border` with CSS
//   Backgrounds 3 initial values: medium (3px) width, none style, and
//   currentcolor color. Like `raikiri_traits::page::PageBox::new`, this gives
//   the two construction patterns zero-argument `new()` and mutation of all
//   public fields (patterns 1 and 2 of the three acceptance patterns in
//   `crates/raikiri/tests/external_consumer.rs`). A builder (pattern 3) was
//   deliberately omitted for this simple three-field value, as for similar
//   structs. `raikiri::Border::new()` and optional field assignment now
//   provide a public value-acquisition path through the umbrella crate.
// - `BorderStyle` and `BorderColor` were also re-exported below. Both are
//   `#[non_exhaustive]` enums; unlike structs, constructing an existing enum
//   variant is not subject to E0639 (as with the `LineHeight` enum). No
//   additional constructor is needed.
//
// **Remaining note on shorthand expansion:** Parsing always expands `border`
// into 12 longhands (four sides × three subproperties) in
// `crate::rule::expand_border`. `crate::rule::expand_shorthand_into` performs
// this expansion at both the parse exit (`parse_declaration_block`) and the
// `@page` cascade entrance (as documented on
// `absolutize_in_page_context_shorthand_fall_throughs` in
// `crates/raikiri-style/src/page.rs`). Thus neither DOM cascade
// (`RuleTree::style_rules()...declarations()`) nor `@page` cascade
// (`raikiri_style::page::cascade_page` / `PageCascadeResult`, neither of
// which the umbrella re-exports) exposes a Declaration containing
// `PropertyValue::Border(Sides<Border>)`. Consumers instead see expanded
// per-side longhands such as `PropertyValue::BorderTopWidth(Length)`,
// `BorderTopStyle(BorderStyle)`, and `BorderTopColor(BorderColor)`. All of
// Border's field types are exported, but real declarations do not currently
// appear in the unexpanded form. `Border::new()` closes a separate gap:
// consumers could name the type but could not construct a value. It does
// not change shorthand expansion.
// cov:ignore: public re-export declarations have no executable body
pub use raikiri_style::{
    AtRuleBody, AtRuleRecord, Atom, CascadeLimitKind, CascadeLimits, CascadeOptions, CascadeResult,
    ComputedBorder, ComputedLength, ComputedLengthPercentage, ComputedLengthPercentageOrAuto,
    ComputedLineHeight, ComputedValues, CssColor, CssRule, CssRuleKind, DisplayValue,
    FontFamilyKind, FontFamilyName, Length, LengthOrAuto, MediaContext, MediaType, Origin,
    PageBleed, PageCascadeResult, PageContextQuery, PageInheritance, PageMarginBoxCascadeResult,
    PageMarginBoxSlot, PageMarks, PageOrientation, PageSize, PageSizeKeyword, PropertyValue,
    QualifiedRuleRecord, RuleNode, RuleTree, RuleTreeLimits, Sides, cascade_with_media_context,
    cascade_with_media_context_for_page, cascade_with_options,
};
// `Border`, `BorderColor`, `BorderStyle`, and `LineHeight` are not exported
// from the raikiri-style crate root. They are public only through
// `raikiri_style::property::{Border, BorderColor, BorderStyle, LineHeight}`,
// so import them separately, rather than in the block above. `BorderColor`
// and `BorderStyle` let consumers read the other two Border fields with
// explicit types.
pub use raikiri_style::property::{Border, BorderColor, BorderStyle, LineHeight};
pub use raikiri_style::{ConsumerPropertyGrammar, ConsumerPropertyRegistration};

// ── url: concrete type for `ParseOptions.base_url: Option<Url>` ────────
pub use url::Url;

// ── bytes: type used by `FetchedResource.bytes: Bytes` / `Body::Bytes(Bytes)` ────
pub use bytes::Bytes;
