//! raikiri — umbrella facade, re-exports, and internal dogfooding APIs.
//!
//! umbrella-facade slice。Consumer が単一 `raikiri` crate だけを dep に
//! 追加すれば HTML parse → cascade された ComputedValues まで得られるように
//! sub-crate から必要な type / trait / function を re-export する。document
//! parse / cascade orchestration / page-streaming render の entry point は
//! `raikiri-html` にあり、本 crate はそれを re-export するだけである。ページ
//! 出力については `PageScene` / `PageDrawables` を dogfooding / validation 用に
//! 提供するが、外部 consumer 向け contract の中心は `raikiri-html` /
//! `raikiri-dom` にある。
//!
//! # Example
//!
//! ```
//! use raikiri::{build_cascaded, parse, ParseOptions};
//!
//! let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
//! let doc = parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
//! let result = build_cascaded(&doc);
//! assert!(!result.computed.is_empty(), "cascade populates per-node ComputedValues");
//! ```

#![allow(rustdoc::private_intra_doc_links)]
pub use raikiri_html::{
    DEFAULT_MAX_AGGREGATE_RESOURCE_BYTES, DEFAULT_MAX_RESOURCE_BYTES, HtmlDocument, RenderOptions,
    RenderResources, ResourceLimits, build_cascaded, build_cascaded_for_page,
    build_cascaded_with_consumer_properties, build_cascaded_with_media_context,
    build_cascaded_with_media_context_for_page,
    build_cascaded_with_media_context_for_page_and_consumer_properties, build_rule_tree,
    build_rule_tree_with_consumer_properties, parse_html, parse_html_with_limits,
    parse_html_with_resources, plan, render_streaming,
};

mod font_context;
pub use font_context::{
    BundledFont, FontContextBuildError, FontContextBuilder, MAX_BUNDLED_FONT_BYTES,
};

mod html_to_png;
pub use html_to_png::{html_to_png, html_to_png_with_fonts, html_to_png_with_resolver};

// ── PageScene + PageDrawables dogfooding surface ───────
// 実装 body は placeholder (empty struct + Default) から段階的に拡張中。
// これは raikiri crate 内の dogfooding / validation 用 pub type surface であり、
// fulgur-facing page output contract は raikiri-dom を中心に定義する。
// `NodeId` は既存 `raikiri_traits::NodeId` (re-export 済み) を再利用し
// PageScene と Document 間で node identity を統一する。
mod page_scene;
pub use page_scene::{
    Fragment, Orientation, PageMetadata, PageScene, Pt, build_page_scene,
    build_page_scene_for_page, build_page_scene_for_page_named,
};

mod page_drawables;
pub use page_drawables::{PageDrawables, TrackedMap};

mod entries;
pub use entries::{
    BlockEntry, BookmarkAnchorEntry, ImageEntry, LinkSpanEntry, ListItemEntry, MulticolRuleEntry,
    ParagraphEntry, SemanticEntry, SvgEntry, TableEntry, TransformEntry,
};

// ── VRT font check API ────────────────────────────────────────────────────
// External consumer が `raikiri` 単独 dep で pinned `FontContext` を build
// できるように、`html_to_png_with_fonts` の依存型を umbrella 経由で公開。
// これが無いと consumer は raikiri-dom / parley を direct dep しなければ
// ならず、実装 crate 依存が漏れる。
pub use parley::FontContext;
pub use raikiri_dom::{FontError, PageMargins, PageSlice, build_wpt_font_ctx, first_page_name};

// ── raikiri-traits: shared vocabulary + DOM traits + error taxonomy ────
// Network API (Request / FetchedResource / NetworkError / Method / Body /
// HeaderMap / AbortSignal / AbortController / ResourceKind) は `NetworkProvider`
// を Consumer 側で implement する際に必須 (fetch signature の param / return type)。
// これらを揃えて re-export することで sub-crate 直接 dep 不要にする。
#[rustfmt::skip]
pub use raikiri_traits::{
    // ── 既存 ──
    AbortController, AbortSignal, Body, CascadeError, ConsumerPropertyEvent,
    ConsumerPropertyObserver, ConsumerPropertyValue, DecodedImage, Dom, Element, FetchedResource,
    HeaderMap, ImagePixelSource, Method, NetworkError, NetworkProvider,
    Node, NodeId, NodeKind, ParseError, QuirksMode, RenderError, RenderWarning,
    Request, ResourceKind, StylesheetKind, WarningKind,

    // ── error / status 系 ──
    RenderStatus, RenderSummary, LimitKind, UnresolvedTarget, UnresolvedReason,
    EmittedSlotInfo, TargetSlotId, TargetKind, TargetDiscrepancy, ExhaustionPolicy,

    // ── plan mode ──
    DocumentPlan, PageSummary, BreakReason, TargetDefinition,

    // ── config ──
    RenderLimits, RenderLimitsBuilder,
    LookaheadConfig, LookaheadConfigBuilder,
    PlanConfig, PlanConfigBuilder,
    StreamingConfig, StreamingConfigBuilder,
    BatchConfig, BatchConfigBuilder,

    // ── paged model ──
    PageBox, PageContext, PageFragment, PageFragmentEvent, PageFragmentPageGeometry,
    PageFragmentLink, PageFragmentLinkEvent,
    PageDefaults, PageDefaultsBuilder,
    LayoutBuffer, TargetRegistry, RunningTemplate, FormData,
    GcpmDirective, ContentValueItem,

    // ── neutral paint payload ──
    PagePaintKind, PagePaintOperation, PagePaintPayload, PaintBorder, PaintBorderStyle, PaintClip,
    PaintColor, PaintFill, PaintGlyph, PaintGlyphRun, PaintImage, PaintInsets, PaintRect,
    PaintResource, PaintResourceBundle, PaintResourceId, PaintResourceKind, PaintShadow,
    PaintTransform,

    // ── traits (Consumer が implement) ──
    PageEventObserver, PagePaintSink, RenderSink, ReplacedResolver, ResourcePolicy,

    // ── strategy traits ──
    LookaheadPolicy, TargetResolver, EmissionPolicy, ReflowPolicy,
    ReflowAction, ContainerOverflowFallback, DirtyDeadline,
    ProbeContext, TargetRequest, ResolvedTarget,

    // ── resolver 補助 ──
    IntrinsicBox, ResolvedIntrinsic, ResolveDisposition,
    ResolverRequest, ResolverError,

    // ── policy 補助 ──
    PolicyViolation, ViolationType,

    // ── layout 補助 ──
    LayoutError,

    // ── symbol ──
    Symbol,
};

// ── raikiri-html: parse pipeline entry ─────────────────────────────────
pub use raikiri_html::{MINIMAL_UA_CSS, ParseOptions, UncascadedDocument, parse};

// ── raikiri-dom: Document (raikiri-html::UncascadedDocument.dom の実体型) ──
// Consumer が `&raikiri::Document` を名指しで受けたい場合に必要。
pub use raikiri_dom::Document;

// ── raikiri-style: cascade pipeline output + value 型 ──────────────────
// ComputedValues field の型は Consumer が読み書きに直接名指しするため、value 系も
// re-export する。
//
// `ComputedValues` の length 系 field が **computed value 層**の型になったため、
// `Computed*` 5 型を追加した。これらは computed 層の leaf 型である
// (`ComputedBorder` のみ width/style/color の 3-field struct で scalar ではない):
//
// - `ComputedLength`                  — `font_size` / `ComputedBorder::width()`
// - `ComputedLengthPercentage`        — `padding` の各 side の leaf 型
// - `ComputedLengthPercentageOrAuto`  — `margin` の各 side / `width` / `height` の leaf 型
// - `ComputedLineHeight`              — `line_height`
// - `ComputedBorder`                  — `border` の各 side の leaf 型
//
// **訂正**: 上記 5 型は leaf 型に過ぎず、`padding` /
// `margin` / `border` の実 field 型は `Sides<ComputedLengthPercentage>` 等の
// 4-side container だった。この最初の re-export 時点では `Sides<T>` 自体が
// re-export されていなかったため、この 3 field は leaf 型を揃えても依然として
// 型付きで名指しできていなかった — その時点ではこれを承知の上で `Sides` を
// approved surface の外と判定し、意図的に見送っていた。後続の修正で
// `Sides` を追加し、この gap を閉じた (下記)。
//
// `Length` (specified 層) は**残す** — `ComputedValues` の field 型ではなくなった
// が、同じく re-export している `PropertyValue` は variant payload に `Length` を
// 持ち続ける (`PropertyValue::FontSize(Length)` 等) ので、Consumer が declaration
// を読むには依然として名指しが要る。外すと `PropertyValue` を扱う Consumer が
// 孤立する。
//
// `Length` が payload に居ることは「その値が specified 層である」ことを意味しない
// — **どの層かは PropertyValue をどこから受け取ったかで決まる**。この規則
// 自体の canonical な記述は
// `raikiri_style::Length` の doc の「本型は『specified 層』を意味しない —
// 層は出所で決まる」節にある (`Length` は下で re-export しているので Consumer
// から到達可能)。出所ごとの内訳:
//
// - `RuleTree` / `Declaration` 由来 (parse 直後の cascade 入力) は specified 層。
//   `Em` / `Rem` / `Pt` / `Percent` がそのまま入る。
// - `raikiri_style::cascade_page` の `PageCascadeResult.declarations` 由来は
//   **computed 層** — `Length` はその computed 値の運搬 shape として使われて
//   いる。**何が保証され例外が何かの canonical な記述は
//   `raikiri_style::page::PageCascadeResult::declarations` の doc** であり、
//   ここで再掲しない (過去に再掲した記述が実装から drift した経緯がある
//   ため)。
//   (`cascade_page` 自体は umbrella が re-export していないので、この経路に
//   届く Consumer は raikiri-style へ直接 dep している場合のみ。)
//
// `Sides<T>` / `LengthOrAuto` / `LineHeight` / `Border` — 追加された。**この
// 4 型追加が上の「最初の re-export 時点では見送った」判断を上書きする** —
// 旧文面をそのまま残すと「追加しない」という嘘が残るため書き換えた。役割:
//
// - `Sides<T>` — layer-agnostic な 4-side container (padding / margin /
//   border の top/right/bottom/left)。specified 層
//   (`PropertyValue::Padding(Sides<Length>)` 等) と computed 層
//   (`ComputedValues.padding: Sides<ComputedLengthPercentage>` 等) の両方で
//   型パラメータ化されて使われる — 他 3 型と違い「specified 層の型」ではない。
//   上段で訂正した「leaf 型はあるが container が無い」gap を本追加が閉じる。
// - `LengthOrAuto` — specified 層。`PropertyValue::MarginTop(LengthOrAuto)` 等
//   の payload。computed 層対応は `ComputedLengthPercentageOrAuto` (上に既出)。
// - `LineHeight` — specified 層。`PropertyValue::LineHeight(LineHeight)` の
//   payload。computed 層対応は `ComputedLineHeight` (上に既出)。
// - `Border` — specified 層。`PropertyValue::Border(Sides<Border>)` の
//   payload。computed 層対応は `ComputedBorder` (上に既出)。`raikiri_style`
//   crate root では re-export されておらず `raikiri_style::property::Border`
//   経由でのみ public なため、下の一括 `pub use` block には含めず、直後の別
//   `pub use` 文でその path から明示 import する (`LineHeight` も同じ理由で同居)。
//
// **さらなる修正**: 上の Sides / Border 追加時点の記述には 2 つの gap が
// あった。(1) `Border` 自体が
// `#[non_exhaustive]` struct のため raikiri crate から struct-literal 構築が
// できず (E0639)、再 export しても値を得る public な経路が無かった —
// `Sides::all` は既存の `Border` 値を 4 面に複製するだけで、その入力自体
// (`Border` 値そのもの) を得る手段ではなかった。(2) `style: BorderStyle` /
// `color: BorderColor` (`Border` の残り 2 field の型) が re-export されて
// おらず、Consumer が `Border::width` 以外の field を型付きで読めなかった。
//
// この修正で両方を埋めた:
//
// - `raikiri_style::property::Border` に `pub fn new() -> Self`
//   (= `Self::default()` の thin wrapper) + `impl Default for Border`
//   (CSS Backgrounds 3 初期値: `width` = medium(3px) / `style` = `none` /
//   `color` = `currentcolor`) を追加。`raikiri_traits::page::PageBox::new`
//   と同じ「zero-arg `new()` + 全 field `pub` による mutation」の 2-pattern
//   契約 (`crates/raikiri/tests/external_consumer.rs` の "3 pattern"
//   acceptance criteria の pattern 1 + pattern 2) — 3 field のみの単純な値
//   なので pattern 3 (builder) は他の類似 struct 同様見送り。これで
//   `raikiri::Border::new()` (+ 必要なら pub field への直接代入) が
//   umbrella 経由の public な value-acquisition path になった。
// - `BorderStyle` / `BorderColor` を re-export に追加 (下記)。どちらも
//   `#[non_exhaustive]` enum だが、struct とは違い既存 variant の直接
//   construct は enum では E0639 の対象外 (`LineHeight` enum と同じ扱い、
//   上の該当箇所参照) — 追加 constructor は不要で re-export のみで足りる。
//
// **shorthand 展開についての残る注記**: `border` shorthand は parse 時に
// 必ず 12 longhand (4 side × 3 sub-property) へ展開される
// (`crate::rule::expand_border`)。この展開は `crate::rule::expand_shorthand_into`
// (parse 出口 `parse_declaration_block` と `@page` cascade 入口の両方から
// 呼ばれる、`crates/raikiri-style/src/page.rs` の
// `absolutize_in_page_context_shorthand_fall_throughs` doc が明記) で行われる
// ため、**DOM cascade (`RuleTree::style_rules()...declarations()`) と `@page`
// cascade (`raikiri_style::page::cascade_page` / `PageCascadeResult`、
// どちらも umbrella は re-export していない) の両方**で
// `PropertyValue::Border(Sides<Border>)` を保持した `Declaration` は観測されず、
// 観測されるのは展開後の `PropertyValue::BorderTopWidth(Length)` /
// `BorderTopStyle(BorderStyle)` / `BorderTopColor(BorderColor)` 等の per-side
// longhand である (`Border` 型は fields としては全部揃うが、実 declaration が
// その形で出てくる経路は現状無い)。`Border::new()` が閉じるのはそれとは別の
// gap — 「型は名指しできるが値を一切構築できない」という construction-path
// gap であり、shorthand 展開の挙動そのものは変えていない。
pub use raikiri_style::{
    AtRuleBody, AtRuleRecord, Atom, CascadeResult, ComputedBorder, ComputedLength,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto, ComputedLineHeight, ComputedValues,
    CssColor, CssRule, CssRuleKind, DisplayValue, Length, LengthOrAuto, MediaContext, MediaType,
    Origin, PageBleed, PageCascadeResult, PageContextQuery, PageInheritance,
    PageMarginBoxCascadeResult, PageMarginBoxSlot, PageMarks, PageOrientation, PageSize,
    PageSizeKeyword, PropertyValue, QualifiedRuleRecord, RuleNode, RuleTree, Sides,
    cascade_with_media_context, cascade_with_media_context_for_page,
};
// `Border` / `BorderColor` / `BorderStyle` / `LineHeight` は raikiri-style
// crate root では re-export されておらず
// (`raikiri_style::property::{Border, BorderColor, BorderStyle, LineHeight}`
// 経由でのみ public)、上の一括 block には含められない (理由は上のコメント
// 参照)。`BorderColor` / `BorderStyle` は追加された
// (`Border` の残り 2 field の型を Consumer に型付きで公開する)。
pub use raikiri_style::property::{Border, BorderColor, BorderStyle, LineHeight};
pub use raikiri_style::{ConsumerPropertyGrammar, ConsumerPropertyRegistration};

// ── url: `ParseOptions.base_url: Option<Url>` の実体型 ─────────────────
pub use url::Url;

// ── bytes: `FetchedResource.bytes: Bytes` / `Body::Bytes(Bytes)` 用 ────
pub use bytes::Bytes;
