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
pub use parse::parse_html;

mod stubs;
pub use stubs::{plan, render_streaming};

// ── raikiri-traits: shared vocabulary + DOM traits + error taxonomy ────
// Network API (Request / FetchedResource / NetworkError / Method / Body /
// HeaderMap / AbortSignal / AbortController / ResourceKind) は `NetworkProvider`
// を Consumer 側で implement する際に必須 (fetch signature の param / return type)。
// これらを揃えて re-export することで sub-crate 直接 dep 不要にする (roborev-refine
// job 230)。
pub use raikiri_traits::{
    AbortController,
    AbortSignal,
    Body,
    CascadeError,
    // Task 6 (stub) 用の最小追加:
    DocumentPlan,
    Dom,
    Element,
    FetchedResource,
    HeaderMap,
    Method,
    NetworkError,
    NetworkProvider,
    Node,
    NodeId,
    NodeKind,
    PageDefaults,
    ParseError,
    PlanConfig,
    QuirksMode,
    RenderError,
    RenderSink,
    RenderStatus,
    RenderWarning,
    ReplacedResolver,
    Request,
    ResourceKind,
    StreamingConfig,
    StylesheetKind,
};

// ── raikiri-html: parse pipeline entry ─────────────────────────────────
pub use raikiri_html::{MINIMAL_UA_CSS, ParseOptions, UncascadedDocument, parse};

// ── raikiri-dom: Document (raikiri-html::UncascadedDocument.dom の実体型) ──
// Consumer が `&raikiri::Document` を名指しで受けたい場合に必要
// (roborev-refine job 228 medium 対応、AC #6 spirit)。
pub use raikiri_dom::Document;

// ── raikiri-style: cascade pipeline output + value 型 ──────────────────
// ComputedValues field の型 (Atom / CssColor / Length) は Consumer が読み書きに
// 直接名指しするため、value 系も re-export (roborev-refine job 228)。
pub use raikiri_style::{
    Atom, CascadeResult, ComputedValues, CssColor, DisplayValue, Length, Origin, PropertyValue,
    RuleTree,
};

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
