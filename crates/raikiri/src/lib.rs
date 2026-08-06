//! raikiri — umbrella crate: primary consumer API and re-exports.
//!
//! M1 の umbrella-facade slice。Consumer が単一 `raikiri` crate だけを dep に
//! 追加すれば HTML parse → cascade された ComputedValues まで得られるように
//! sub-crate から必要な type / trait / function を re-export し、cascade
//! orchestration entry point `build_cascaded` を提供する
//! (spec §M1、raikiri-spike-m1.23)。
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

use raikiri_style::cascade;

mod html_document;
pub use html_document::HtmlDocument;

mod parse;
pub use parse::{parse_html, parse_html_with_limits};

mod stubs;
pub use stubs::{plan, render_streaming};

mod html_to_png;
pub use html_to_png::{html_to_png, html_to_png_with_fonts};

// ── M2 kickoff seed: PageScene + PageDrawables consumer surface ────────
// raikiri-spike-os52 (Sprint 22)。実装 body は placeholder (empty struct + Default)、
// consumer facade の pub type surface のみを landing する。
// `NodeId` は既存 [`raikiri_traits::NodeId`] (m1.23 landed re-export) を再利用し
// PageScene と Document 間で node identity を統一する (coord Option A on
// raikiri-spike-os52、bd comment 参照)。
mod page_scene;
pub use page_scene::{Fragment, Orientation, PageMetadata, PageScene, Pt};

mod page_drawables;
pub use page_drawables::{PageDrawables, TrackedMap};

mod entries;
pub use entries::{
    BlockEntry, BookmarkAnchorEntry, ImageEntry, LinkSpanEntry, ListItemEntry, MulticolRuleEntry,
    ParagraphEntry, SemanticEntry, SvgEntry, TableEntry, TransformEntry,
};

// ── VRT font pin API (raikiri-spike-e93) ────────────────────────────────
// External consumer が `raikiri` 単独 dep で pinned `FontContext` を build
// できるように、`html_to_png_with_fonts` の依存型を umbrella 経由で公開。
// これが無いと consumer は raikiri-dom / parley を direct dep しなければ
// ならず、実装 crate 依存が漏れる (roborev Medium finding e93 round 2)。
pub use parley::FontContext;
pub use raikiri_dom::{FontError, build_wpt_font_ctx};

// ── raikiri-traits: shared vocabulary + DOM traits + error taxonomy ────
// Network API (Request / FetchedResource / NetworkError / Method / Body /
// HeaderMap / AbortSignal / AbortController / ResourceKind) は `NetworkProvider`
// を Consumer 側で implement する際に必須 (fetch signature の param / return type)。
// これらを揃えて re-export することで sub-crate 直接 dep 不要にする (roborev-refine
// job 230)。
#[rustfmt::skip]
pub use raikiri_traits::{
    // ── 既存 (m1.23) ──
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
    PageBox, PageContext, PageFragment,
    PageDefaults, PageDefaultsBuilder,
    LayoutBuffer, TargetRegistry, RunningTemplate, FormData,
    GcpmDirective, ContentValueItem,

    // ── traits (Consumer が implement) ──
    RenderSink, ReplacedResolver, ResourcePolicy,

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
// Consumer が `&raikiri::Document` を名指しで受けたい場合に必要
// (roborev-refine job 228 medium 対応、AC #6 spirit)。
pub use raikiri_dom::Document;

// ── raikiri-style: cascade pipeline output + value 型 ──────────────────
// ComputedValues field の型は Consumer が読み書きに直接名指しするため、value 系も
// re-export (roborev-refine job 228)。
//
// bd raikiri-spike-zls8 (decision raikiri-spike-082k Phase 2) で
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
// **訂正 (bd raikiri-spike-eow8)**: 上記 5 型は leaf 型に過ぎず、`padding` /
// `margin` / `border` の実 field 型は `Sides<ComputedLengthPercentage>` 等の
// 4-side container だった。zls8 時点では `Sides<T>` 自体が re-export されて
// いなかったため、この 3 field は leaf 型を揃えても依然として型付きで名指し
// できていなかった — zls8 はこれを承知の上で `Sides` を approved surface の
// 外と判定し、意図的に見送っている (debt lens 追認済み)。eow8 で `Sides` を
// 追加し、この gap を閉じた (下記)。
//
// `Length` (specified 層) は**残す** — `ComputedValues` の field 型ではなくなった
// が、同じく re-export している `PropertyValue` は variant payload に `Length` を
// 持ち続ける (`PropertyValue::FontSize(Length)` 等) ので、Consumer が declaration
// を読むには依然として名指しが要る。外すと `PropertyValue` を扱う Consumer が
// 孤立する。
//
// `Length` が payload に居ることは「その値が specified 層である」ことを意味しない
// — **どの層かは PropertyValue をどこから受け取ったかで決まる** (bd
// raikiri-spike-sshp)。この規則自体の canonical な記述は
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
//   ここで再掲しない (再掲は既に 2 度 drift した — bd raikiri-spike-awjx)。
//   (`cascade_page` 自体は umbrella が re-export していないので、この経路に
//   届く Consumer は raikiri-style へ直接 dep している場合のみ。)
//
// `Sides<T>` / `LengthOrAuto` / `LineHeight` / `Border` — bd raikiri-spike-eow8
// (2026-08-07 PMO approve、wall/umbrella add 方向) で追加。**この 4 型追加が
// 上の「zls8 時点では見送った」判断を上書きする** — 旧文面をそのまま残すと
// 「追加しない」という嘘が残るため書き換えた。役割:
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
//   また `Border` 自体が `#[non_exhaustive]` struct なので raikiri crate から
//   struct-literal 構築はできない (E0639)。保持/生成の public な経路は現状
//   存在しない — `Sides::all` は既存の `Border` 値を 4 面に複製するだけで、
//   その入力自体 (`Border` 値そのもの) を得る手段ではない。
//
// `Border` は `width: Length` field のみ Consumer が型付きに読める。
// `style: BorderStyle` / `color: BorderColor` (どちらも `raikiri_style::property`
// では public、`raikiri_style::property::Border` と同じ経路で到達可能) は本
// task の approved scope (eow8 PMO 承認は Sides/LengthOrAuto/LineHeight/Border
// の 4 型に限定) 外のため未 re-export — 追加するには別の wall/umbrella add
// 方向 escalation が要る。follow-up: bd raikiri-spike-x0dq (`Border` 自体、
// shorthand 展開により実 CSS からは値が得られない事情も含めて記録)。
pub use raikiri_style::{
    Atom, CascadeResult, ComputedBorder, ComputedLength, ComputedLengthPercentage,
    ComputedLengthPercentageOrAuto, ComputedLineHeight, ComputedValues, CssColor, DisplayValue,
    Length, LengthOrAuto, Origin, PropertyValue, RuleTree, Sides,
};
// `Border` / `LineHeight` は raikiri-style crate root では re-export されておらず
// (`raikiri_style::property::{Border, LineHeight}` 経由でのみ public)、上の一括
// block には含められない (理由は上のコメント参照)。
pub use raikiri_style::property::{Border, LineHeight};

// ── url: `ParseOptions.base_url: Option<Url>` の実体型 ─────────────────
pub use url::Url;

// ── bytes: `FetchedResource.bytes: Bytes` / `Body::Bytes(Bytes)` 用 ────
pub use bytes::Bytes;

/// UA + Consumer 提供 stylesheet を Document から取り出し、Origin を割り当てて
/// RuleTree を組み、raikiri-html が parse 時に集約した head 配下の `<style>`
/// element の text を Author として追加した上で cascade を実行する umbrella
/// orchestration entry point (spec §M1.4a、raikiri-spike-m1.23)。
///
/// M1 では `raikiri_style::cascade` は常に `Ok` を返すため、内部で `expect` する
/// (M2+ で Result 反映を検討)。
///
/// Consumer は `raikiri_html::parse` → `raikiri::build_cascaded` の 2 step だけで
/// per-node ComputedValues を得られる。
///
/// # DOM `<style>` の集約 scope (M1 contract)
///
/// `UncascadedDocument::stylesheet_sources` を Author として消費する。
/// この Vec は parse 時に [`raikiri_html::parse`] 内の `extract_inline_stylesheets`
/// が **head 配下** の `<style>` element の text を document order で集約し、
/// `<template>` subtree は spec §14.1 の inertness に従って skip 済み。
/// `<body>` 内の `<style>` は position-aware semantics が必要なため M1 では
/// 未対応 (M2+ で拡張予定、`raikiri-html/src/sink.rs::extract_inline_stylesheets`
/// の invariant に一致、roborev-refine job 226 参照)。
///
/// # source_order tie-break (Author vs Author)
///
/// `Document.stylesheets()` (parse 時に注入された UA + `extra_stylesheets`) が
/// 先に RuleTree に流し込まれ、次に `stylesheet_sources` (head 配下 `<style>`)
/// が Author として追加される。同 Author 内の tie-break (同 specificity・同
/// `!important`) では後から来た方が source_order 大で勝つため、**DOM `<style>`
/// は `extra_stylesheets` を上書きする**。この precedence は仕様書 §M1.4a には
/// 明記されていない M1 実装判断 (raikiri-spike-m1.23)。
///
/// # Dep 方向
///
/// `StylesheetKind → Origin` の翻訳は raikiri-html にも raikiri-style にも置かず、
/// umbrella (本 crate) 内で明示的に書く。これにより下位 crate 間の逆依存を発生
/// させない (raikiri-spike-m1.23 Acceptance #2)。
pub fn build_cascaded(doc: &UncascadedDocument) -> CascadeResult {
    let mut tree = RuleTree::empty();

    // Document に associate されている全 stylesheet を kind に応じて Origin
    // に map。呼び出し順 (=注入順) が cascade の source_order を決める。
    for (source, kind) in doc.dom.stylesheets() {
        let origin = stylesheet_kind_to_origin(kind);
        tree.add_stylesheet(source, origin);
    }

    // raikiri-html が parse 時に head 配下 / template-inert filter 越しに集約した
    // `<style>` element の text を Author として追加。<body> style は M1 未対応
    // (roborev-refine job 226 medium 対応)。
    for source in &doc.stylesheet_sources {
        tree.add_stylesheet(source, Origin::Author);
    }

    cascade(&doc.dom, &tree).expect("m1 では cascade は常に Ok")
}

/// dom-level の [`StylesheetKind`] (raikiri-traits) を cascade-level の
/// [`Origin`] (raikiri-style) に翻訳。dep 方向を保つため umbrella 内で保持。
///
/// `StylesheetKind` は他 crate の `#[non_exhaustive]` enum のため exhaustive match
/// はできないが、将来 variant が追加された場合の silent misroute を防ぐため
/// `_` arm は `unreachable!` で loud fail させる (M1 では UserAgent / Author の 2
/// variant で網羅済み)。
fn stylesheet_kind_to_origin(kind: StylesheetKind) -> Origin {
    match kind {
        StylesheetKind::UserAgent => Origin::UserAgent,
        StylesheetKind::Author => Origin::Author,
        // `StylesheetKind` は `#[non_exhaustive]`。M1 では UserAgent / Author の 2 variant を
        // 上で網羅済み。将来 User 等が追加された時点で対応が漏れるとここに到達し、
        // silent misroute を防ぐため panic で loud fail する (dev が cascade origin map の
        // 更新に気付ける)。
        _ => unreachable!(
            "StylesheetKind variant not yet mapped to Origin — update stylesheet_kind_to_origin in raikiri crate (m1.23)"
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
            "cascade.computed.len() must equal document.node_count() (m1.23 contract)"
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
        // 2 経路の cascade が同じ結果を出すことを pin (parse_html は
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

    /// M1 では replaced element なし → resolve が呼ばれない前提で unreachable。
    struct NoopResolver;
    impl ReplacedResolver for NoopResolver {
        fn resolve(
            &self,
            _req: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            unreachable!("plan/render_streaming stubs must not call resolver")
        }
    }

    /// M1 sink stub。accept_page / finish_render は No-op。stub は sink を呼ばない前提。
    struct NoopSink;
    impl RenderSink for NoopSink {
        fn accept_page(
            &mut self,
            _fragment: raikiri_traits::PageFragment,
        ) -> Result<(), std::io::Error> {
            unreachable!("render_streaming stub must not call sink")
        }
        fn finish_render(
            &mut self,
            _summary: raikiri_traits::RenderSummary,
        ) -> Result<(), std::io::Error> {
            unreachable!("render_streaming stub must not call sink")
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
        .expect_err("plan stub must return Err");
        match err {
            RenderError::Unimplemented { feature, .. } => {
                assert_eq!(feature, "plan", "feature must identify plan API");
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    #[test]
    fn render_streaming_returns_unimplemented_with_feature_name() {
        let doc = hello_world_doc();
        let mut sink = NoopSink;
        let err = render_streaming(
            &doc,
            PageDefaults::default(),
            &NoopResolver,
            StreamingConfig::default(),
            &mut sink,
        )
        .expect_err("render_streaming stub must return Err");
        match err {
            RenderError::Unimplemented {
                feature,
                migration_hint,
            } => {
                assert_eq!(feature, "render_streaming", "feature must identify API");
                assert!(
                    migration_hint.contains("html_to_png"),
                    "hint must point Consumer to html_to_png, got {migration_hint:?}"
                );
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }
}
