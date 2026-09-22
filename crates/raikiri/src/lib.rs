//! raikiri — umbrella facade, re-exports, and internal dogfooding APIs.
//!
//! umbrella-facade slice。Consumer が単一 `raikiri` crate だけを dep に
//! 追加すれば HTML parse → cascade された ComputedValues まで得られるように
//! sub-crate から必要な type / trait / function を re-export し、cascade
//! orchestration entry point `build_cascaded` を提供する。ページ出力については
//! `PageScene` / `PageDrawables` を dogfooding / validation 用に提供するが、
//! fulgur-facing contract の中心は `raikiri-dom` にある。
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
mod html_document;
pub use html_document::HtmlDocument;

mod parse;
pub use parse::{parse_html, parse_html_with_limits};

mod stubs;
pub use stubs::{plan, render_streaming, render_streaming_with_observer};

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
    AbortController, AbortSignal, Body, CascadeError, Dom, Element,
    FetchedResource, HeaderMap, Method, NetworkError, NetworkProvider,
    Node, NodeId, NodeKind, ParseError, QuirksMode, RenderError, RenderWarning,
    Request, ResourceKind, StylesheetKind,

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

    // ── traits (Consumer が implement) ──
    PageEventObserver, RenderSink, ReplacedResolver, ResourcePolicy,

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

// ── url: `ParseOptions.base_url: Option<Url>` の実体型 ─────────────────
pub use url::Url;

// ── bytes: `FetchedResource.bytes: Bytes` / `Body::Bytes(Bytes)` 用 ────
pub use bytes::Bytes;

/// UA + Consumer 提供 stylesheet を Document から取り出し、Origin を割り当てて
/// RuleTree を組み、raikiri-html が parse 時に集約した head 配下の `<style>`
/// element の text を Author として追加した上で cascade を実行する umbrella
/// orchestration entry point。
///
/// 現状 `raikiri_style::cascade` は常に `Ok` を返すため、内部で `expect` する
/// (将来 Result 反映を検討)。
///
/// Consumer は `raikiri_html::parse` → `raikiri::build_cascaded` の 2 step だけで
/// per-node ComputedValues を得られる。
///
/// # DOM `<style>` の集約 scope
///
/// `UncascadedDocument::stylesheet_sources` を Author として消費する。
/// この Vec は parse 時に [`raikiri_html::parse`] 内の `extract_inline_stylesheets`
/// が **head 配下** の `<style>` element の text を document order で集約し、
/// `<template>` subtree は spec §14.1 の inertness に従って skip 済み。
/// `<body>` 内の `<style>` は position-aware semantics が必要なため現状
/// 未対応 (将来拡張予定、`raikiri-html/src/sink.rs::extract_inline_stylesheets`
/// の invariant に一致)。
///
/// # DOM `<style>` (Author) vs `extra_stylesheets` (User)
///
/// `Document.stylesheets()` (parse 時に注入された UA + `extra_stylesheets`) が
/// 先に RuleTree に流し込まれ、次に `stylesheet_sources` (head 配下 `<style>`)
/// が Author として追加される。従来は `extra_stylesheets`
/// も `Author` としてタグされており、DOM `<style>` との勝敗は同一 origin 内の
/// source_order tie-break (後から来た方が勝つ) に依存していた。その後
/// `extra_stylesheets` は [`Origin::User`] に retag された
/// ため、両者はもはや同一 origin ではない — 勝敗は origin rank の差で
/// specificity / source_order を問わず決まる。
///
/// **normal 同士なら** [`Origin::Author`] (normal rank 3) > [`Origin::User`]
/// (normal rank 1) なので **DOM `<style>` が `extra_stylesheets` を上書きする**
/// — 旧実装判断が偶然同一 origin tie-break で
/// 実現していたのと同じ勝敗だが、根拠が「同 origin tie-break」から「別
/// origin の rank 差」に変わった。
///
/// **`!important` が絡むとこの勝敗は反転しうる** (CSS Cascading L4 §6.3 の
/// importance による origin 順反転)。`extra_stylesheets` 側が `!important`
/// を持てば ([`Origin::User`] important rank 6) DOM `<style>` 側の
/// importance に関係なく (`Author` は normal rank 3 / important rank 4、
/// いずれも 6 未満) `extra_stylesheets` が勝つ。逆に `extra_stylesheets` 側
/// が normal (rank 1) なら DOM `<style>` は normal/important いずれでも
/// (rank 3 / 4、いずれも 1 より上) 勝つ — 実質、勝敗は `extra_stylesheets`
/// 側の importance だけで決まる。
///
/// # Dep 方向
///
/// `StylesheetKind → Origin` の翻訳は raikiri-html にも raikiri-style にも置かず、
/// umbrella (本 crate) 内で明示的に書く。これにより下位 crate 間の逆依存を発生
/// させない。
pub fn build_cascaded(doc: &UncascadedDocument) -> CascadeResult {
    build_cascaded_with_media_context(doc, &MediaContext::default())
}

/// Build the cascade for one page-context query using the default media
/// context.
pub fn build_cascaded_for_page(
    doc: &UncascadedDocument,
    page_query: &PageContextQuery,
) -> CascadeResult {
    build_cascaded_with_media_context_for_page(doc, &MediaContext::default(), page_query)
}

/// Build the rule tree and run the cascade for an explicit media context.
///
/// [`build_cascaded`] remains the compatibility entry point and uses the
/// default paged (`print`) context.
pub fn build_cascaded_with_media_context(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
) -> CascadeResult {
    build_cascaded_with_media_context_for_page(doc, media_context, &PageContextQuery::default())
}

/// Build the element and `@page` cascades for one page-context query.
///
/// The first-page render path uses this entry point with `is_first` and
/// `is_right` set. A future page-stream driver can call it once per page with
/// the page name and pseudo-page state selected by its break algorithm.
pub fn build_cascaded_with_media_context_for_page(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
) -> CascadeResult {
    let tree = build_rule_tree(doc);
    cascade_with_media_context_for_page(&doc.dom, &tree, media_context, page_query)
        .expect("cascade は常に Ok のはず")
}

/// Build the stylesheet rule tree used by the umbrella cascade.
///
/// Keeping this operation separate lets a paged renderer retain the parsed
/// `@page` rules while it performs a per-page cascade in a later page loop.
pub fn build_rule_tree(doc: &UncascadedDocument) -> RuleTree {
    let mut tree = RuleTree::empty();

    // Document に associate されている全 stylesheet を kind に応じて Origin
    // に map。呼び出し順 (=注入順) が cascade の source_order を決める。
    for (source, kind) in doc.dom.stylesheets() {
        let origin = stylesheet_kind_to_origin(kind);
        tree.add_stylesheet(source, origin);
    }

    // raikiri-html が parse 時に head 配下 / template-inert filter 越しに集約した
    // `<style>` element の text を Author として追加。<body> style は現状未対応。
    for source in &doc.stylesheet_sources {
        tree.add_stylesheet(source, Origin::Author);
    }

    tree
}

/// dom-level の [`StylesheetKind`] (raikiri-traits) を cascade-level の
/// [`Origin`] (raikiri-style) に翻訳。dep 方向を保つため umbrella 内で保持。
///
/// `StylesheetKind` は他 crate の `#[non_exhaustive]` enum のため exhaustive match
/// はできないが、将来 variant が追加された場合の silent misroute を防ぐため
/// `_` arm は `unreachable!` で loud fail させる (現時点で
/// UserAgent / User / Author の 3 variant で網羅済み)。
fn stylesheet_kind_to_origin(kind: StylesheetKind) -> Origin {
    match kind {
        StylesheetKind::UserAgent => Origin::UserAgent,
        StylesheetKind::User => Origin::User,
        StylesheetKind::Author => Origin::Author,
        // `StylesheetKind` は `#[non_exhaustive]`。現時点で
        // UserAgent / User / Author の 3 variant を上で網羅済み。将来別の variant が
        // 追加された時点で対応が漏れるとここに到達し、silent misroute を防ぐため
        // panic で loud fail する (dev が cascade origin map の更新に気付ける)。
        // cov:ignore: defensive `_` arm for a cross-crate `#[non_exhaustive]` enum —
        // unreachable by construction while all 3 current variants are matched above;
        // only becomes reachable if a future variant is added upstream without a
        // corresponding arm here (the panic message tells the dev to add one).
        _ => unreachable!(
            "StylesheetKind variant not yet mapped to Origin — update stylesheet_kind_to_origin in raikiri crate"
        ),
    }
}

/// Cascade orchestration が Document 内 `<style>` を Author 経路で使うこと、および
/// Document.stylesheets 経由の UA CSS がここに二重計上されないことを再確認する
/// smoke-test は tests/build_cascaded.rs 側で担当する。
#[cfg(test)]
mod smoke_tests {
    use super::*;

    #[test]
    fn stylesheet_kind_to_origin_matches_spec() {
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::UserAgent),
            Origin::UserAgent,
        );
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::User),
            Origin::User,
        );
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::Author),
            Origin::Author,
        );
    }
}

#[cfg(test)]
mod html_document_tests {
    use super::*;

    fn hello_world_doc() -> HtmlDocument {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let uncascaded = raikiri_html::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
        let cascade = build_cascaded(&uncascaded);
        // 内部 field 直接 construct (crate-internal test なので pub(crate) field OK)
        HtmlDocument {
            uncascaded,
            cascade,
        }
    }

    #[test]
    fn html_document_accessors_expose_underlying_types() {
        let doc = hello_world_doc();
        // accessor が inner field と identity 一致 (別 heap 割当てなし)
        let dom_ref: &raikiri_dom::Document = doc.dom();
        let cascade_ref: &CascadeResult = doc.cascade();
        let sources_ref: &[String] = doc.stylesheet_sources();

        assert!(
            std::ptr::eq(dom_ref, &doc.uncascaded.dom),
            "dom() must return &doc.uncascaded.dom"
        );
        assert!(
            std::ptr::eq(cascade_ref, &doc.cascade),
            "cascade() must return &doc.cascade"
        );
        assert!(
            std::ptr::eq(
                sources_ref.as_ptr(),
                doc.uncascaded.stylesheet_sources.as_ptr()
            ) || (sources_ref.is_empty() && doc.uncascaded.stylesheet_sources.is_empty()),
            "stylesheet_sources() must alias inner Vec"
        );
    }

    #[test]
    fn html_document_cascade_populated_after_construct() {
        let doc = hello_world_doc();
        assert!(
            !doc.cascade().computed.is_empty(),
            "cascade must be populated (build_cascaded produces per-node ComputedValues)"
        );
        // node_count と cascade.computed.len() 契約
        assert_eq!(
            doc.cascade().computed.len(),
            doc.dom().node_count(),
            "cascade.computed.len() must equal document.node_count()"
        );
    }
}

#[cfg(test)]
mod parse_html_tests {
    use super::*;

    #[test]
    fn parse_html_returns_html_document_with_cascade_populated() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html should succeed");
        assert!(
            !doc.cascade().computed.is_empty(),
            "parse_html output must have populated cascade"
        );
        assert_eq!(
            doc.cascade().computed.len(),
            doc.dom().node_count(),
            "cascade / dom node_count invariant"
        );
    }

    #[test]
    fn parse_html_propagates_parse_error_from_io() {
        struct FailingReader;
        impl std::io::Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
        }
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let err = parse_html(FailingReader, &opts).expect_err("must fail on reader error");
        assert!(
            matches!(err, RenderError::Parse(ParseError::Io(_))),
            "expected RenderError::Parse(ParseError::Io), got {err:?}"
        );
    }

    #[test]
    fn parse_html_propagates_utf8_error() {
        // 0x80 は UTF-8 continuation byte 単独、invalid UTF-8
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let err =
            parse_html(&[0x80u8, 0x80, 0x80][..], &opts).expect_err("must fail on invalid UTF-8");
        assert!(
            matches!(err, RenderError::Parse(ParseError::Encoding { .. })),
            "expected RenderError::Parse(ParseError::Encoding), got {err:?}"
        );
    }

    #[test]
    fn parse_html_baked_cascade_matches_manual_build_cascaded() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        // 2 経路の cascade が同じ結果を出すことを check (parse_html は
        // build_cascaded を内部で呼んでいる契約)
        let via_parse_html = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html");
        let via_manual = {
            let uncascaded = raikiri_html::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
            build_cascaded(&uncascaded)
        };
        assert_eq!(
            via_parse_html.cascade().computed.len(),
            via_manual.computed.len(),
            "parse_html と手動 build_cascaded で cascade node 数が一致"
        );
    }

    // `HtmlDocument.uncascaded` は `pub(crate)` (Consumer 向けには非公開) の
    // ため、`RenderLimits::max_parse_warnings` が実際に parse 中の
    // `RaikiriTreeSink` に consult されていることの検証は、この crate 内部の
    // white-box test としてのみ書ける (`doc.uncascaded.warnings` へのアクセス
    // が必要)。

    #[test]
    fn parse_html_with_limits_consults_custom_max_parse_warnings() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        // `</x>` は closing tag に対応する開始要素が無いため、html5ever の
        // エラー回復アルゴリズムが 1 個あたり概ね 1 個の非致命 parse error を
        // 報告する (raikiri_html crate の同種 test と同じ input shape)。cap=5
        // → last_real_slot=4 なので real warning 4 件 + synthetic 1 件の
        // 計 5 件で頭打ちになる (reserved-last-slot 契約、
        // `RaikiriTreeSink::parse_error` doc 参照)。
        let malformed = b"</x>".repeat(50);

        let strict_limits = RenderLimits::builder().max_parse_warnings(Some(5)).build();
        let strict_doc = parse_html_with_limits(malformed.as_slice(), &opts, strict_limits)
            .expect("malformed input still recovers");
        assert_eq!(
            strict_doc.uncascaded.warnings.len(),
            5,
            "custom max_parse_warnings=5 must be consulted and yield exactly 4 real + 1 synthetic warning"
        );
        match &strict_doc.uncascaded.warnings.last().expect("cap > 0").kind {
            raikiri_traits::WarningKind::HtmlParseError { message } => {
                assert!(
                    message.contains("5-warning cap"),
                    "last entry must be the synthetic suppression notice naming the runtime cap (5), got: {message:?}"
                );
            }
            other => panic!("expected HtmlParseError, got {other:?}"),
        }

        // Contrast: 同じ input を default cap (1024) で parse すると 5 件より
        // 多く記録される — cap 値が実際に RenderLimits から読まれていること
        // (固定の小さい値に偶然収まっただけではないこと) を check する。
        let default_doc =
            parse_html_with_limits(malformed.as_slice(), &opts, RenderLimits::default())
                .expect("malformed input still recovers");
        assert!(
            default_doc.uncascaded.warnings.len() > strict_doc.uncascaded.warnings.len(),
            "default cap must allow strictly more warnings than the custom cap=5, got default={} strict={}",
            default_doc.uncascaded.warnings.len(),
            strict_doc.uncascaded.warnings.len()
        );
    }

    #[test]
    fn parse_html_with_limits_zero_max_parse_warnings_records_one_synthetic_entry() {
        // cap=0 has no slot for a real warning, but must still record exactly
        // one synthetic suppression entry when parse errors actually occur,
        // consistent with cap>0's trip-and-record-suppression semantics.
        // Pins that this boundary is reachable via `RenderLimits`, not just
        // via `RaikiriTreeSink::new()` directly.
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let malformed = b"</x>".repeat(50);

        let limits = RenderLimits::builder().max_parse_warnings(Some(0)).build();
        let doc = parse_html_with_limits(malformed.as_slice(), &opts, limits)
            .expect("malformed input still recovers");
        assert_eq!(
            doc.uncascaded.warnings.len(),
            1,
            "max_parse_warnings=Some(0) must record exactly one synthetic suppression entry, got {}",
            doc.uncascaded.warnings.len()
        );
    }

    #[test]
    fn parse_html_with_limits_none_max_parse_warnings_disables_cap() {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        // 2,000 個の `</x>` は default cap (1024) 下では確実に cap に到達する
        // 入力サイズ (raikiri_html crate の同種 test で確認済み)。
        let malformed = b"</x>".repeat(2_000);

        let mut limits = RenderLimits::default();
        limits.max_parse_warnings = None;
        let doc = parse_html_with_limits(malformed.as_slice(), &opts, limits)
            .expect("malformed input still recovers");

        assert!(
            doc.uncascaded.warnings.len() > 1024,
            "max_parse_warnings=None must not cap warnings at the default 1024, got {}",
            doc.uncascaded.warnings.len()
        );
    }
}

#[cfg(test)]
mod stub_tests {
    use super::*;

    fn hello_world_doc() -> HtmlDocument {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse")
    }

    /// Current focused documents do not contain replaced elements, so this
    /// resolver is expected not to be called.
    struct NoopResolver;
    impl ReplacedResolver for NoopResolver {
        fn resolve(
            &self,
            _req: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            unreachable!("test document has no replaced element")
        }
    }

    /// Recording sink used to pin page emission and completion ordering.
    #[derive(Default)]
    struct RecordingSink {
        pages: usize,
        summary: Option<raikiri_traits::RenderSummary>,
    }
    impl RenderSink for RecordingSink {
        fn accept_page(
            &mut self,
            _fragment: raikiri_traits::PageFragment,
        ) -> Result<(), std::io::Error> {
            self.pages += 1;
            Ok(())
        }
        fn finish_render(
            &mut self,
            summary: raikiri_traits::RenderSummary,
        ) -> Result<(), std::io::Error> {
            self.summary = Some(summary);
            Ok(())
        }
    }

    #[test]
    fn plan_returns_unimplemented_with_feature_name() {
        let doc = hello_world_doc();
        let err = plan(
            &doc,
            PageDefaults::default(),
            &NoopResolver,
            PlanConfig::default(),
        )
        .expect_err("unimplemented plan API must return Err");
        match err {
            RenderError::Unimplemented { feature, .. } => {
                assert_eq!(feature, "plan", "feature must identify plan API");
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    #[test]
    fn render_streaming_emits_pages_and_finishes_once() {
        let doc = hello_world_doc();
        let mut sink = RecordingSink::default();
        let status = render_streaming(
            &doc,
            PageDefaults::default(),
            &NoopResolver,
            StreamingConfig::default(),
            &mut sink,
        )
        .expect("render_streaming should complete");
        let summary = match status {
            RenderStatus::Completed(summary) => summary,
            RenderStatus::Aborted { partial_pages } => {
                panic!("unexpected abort after {partial_pages} pages")
            }
            _ => panic!("unexpected non-exhaustive render status"),
        };
        assert_eq!(sink.pages, 1);
        assert_eq!(summary.total_pages, 1);
        assert_eq!(sink.summary.expect("finish_render").total_pages, 1);
    }

    #[test]
    fn render_streaming_abort_skips_completion() {
        let doc = hello_world_doc();
        let controller = AbortController::new();
        controller.abort();
        let config = StreamingConfig::builder()
            .signal(Some(controller.signal.clone()))
            .build();
        let mut sink = RecordingSink::default();
        let status = render_streaming(
            &doc,
            PageDefaults::default(),
            &NoopResolver,
            config,
            &mut sink,
        )
        .expect("abort is a status, not an error");
        assert!(matches!(status, RenderStatus::Aborted { partial_pages: 0 }));
        assert_eq!(sink.pages, 0);
        assert!(sink.summary.is_none());
    }
}
