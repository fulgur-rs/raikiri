//! Public parse entrypoints.

use std::borrow::Cow;
use std::io::Read;

use html5ever::driver::{ParseOpts, parse_document};
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeSink;
use raikiri_traits::{
    Body, Method, NetworkError, ParseError, RenderWarning, Request, ResourceKind, StylesheetKind,
    WarningKind,
};
use url::Url;

use crate::import::{
    ImportBudget, expand_stylesheet_imports_with_budget, network_error_summary, redacted_url,
    sanitize_policy_violation,
};
use crate::sink::RaikiriTreeSink;
use crate::types::{ParseOptions, UncascadedDocument};

/// HTML を parse し [`UncascadedDocument`] を返す。cascade 前の DOM +
/// inline `<style>` 抽出 + parse warning が含まれる。
///
/// `input` は UTF-8 の byte stream として扱う。Read 失敗は
/// [`ParseError::Io`]、UTF-8 として invalid な入力は [`ParseError::Encoding`]
/// を返す (spike scope。encoding_rs 導入は将来予定)。
///
/// # Example
///
/// ```
/// use raikiri_html::{parse, ParseOptions};
/// use raikiri_traits::Dom;
///
/// let html = b"<html><body>Hi</body></html>";
/// let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
/// let doc = parse(&html[..], &opts).unwrap();
/// // Parse succeeded; dom has root
/// assert_eq!(doc.dom.root_id().0, 0);
/// ```
pub fn parse<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError> {
    parse_with_sink(input, RaikiriTreeSink::default(), options)
}

/// Consumer-supplied sink 経由で parse する。Consumer wrapper は
/// `type Output = UncascadedDocument` を宣言し、`finish(self)` で inner
/// sink の finish 結果を bubble させる契約。
///
/// parse 完了時に既定 UA CSS + `options.extra_stylesheets` を
/// [`raikiri_dom::Document::add_stylesheet`] 経由で Document 状態に注入する
/// (UA=UserAgent/extra=User kind)。Extra と head stylesheet の leading `@import`
/// は `options.network` がある場合に source order に従って展開される。続けて
/// `<head>` 内の external stylesheet を `options.network` / `options.base_url` 経由で
/// fetch し、成功分を `UncascadedDocument.stylesheet_sources` に Author として
/// head order で統合する (`fetch_external_stylesheets` doc 参照)。外部 stylesheet
/// の import は response の `final_url` を nested import の base として使う。
/// 失敗した import は元の at-rule を保持し、fetch failure は
/// `NetworkFallback` / `PolicyWarning` を記録する。
pub fn parse_with_sink<R, S>(
    mut input: R,
    sink: S,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError>
where
    R: Read,
    S: TreeSink<Handle = usize, Output = UncascadedDocument>,
{
    // 2-step: reader failures → Io、UTF-8 conversion failures → Encoding。
    // read_to_string の InvalidData 一括分類 (reader が非-encoding 由来で
    // InvalidData を返すケース) を防ぐ。
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).map_err(ParseError::Io)?;
    let buf = String::from_utf8(bytes).map_err(|e| ParseError::Encoding {
        label: String::from("utf-8"),
        reason: e.to_string(),
    })?;

    let parser = parse_document(sink, ParseOpts::default());
    let mut doc = parser.one(buf.as_str());

    // 既定 UA CSS を Document に注入。
    doc.dom.add_stylesheet(
        Cow::Borrowed(crate::ua::MINIMAL_UA_CSS),
        StylesheetKind::UserAgent,
    );

    // HTML の document base URL は inline / extra stylesheet の relative
    // `@import` 解決にも使う。外部 stylesheet 自身は fetch 後の
    // `FetchedResource::final_url` を base に使う。
    let effective_base = effective_document_base_url(&doc, options.base_url.as_ref());
    // Share import limits across every stylesheet root in this document. A
    // separate per-root expander would let many inline/link sheets multiply
    // the fetch and expansion caps.
    let mut import_budget = ImportBudget::default();

    // Consumer 提供の extra_stylesheets を User origin として追加
    // (ParseOptions::extra_stylesheets の実 consume 経路)。
    // StylesheetKind::Author retag から独立 StylesheetKind::User へ移行済み —
    // real author-origin stylesheet (`<link rel=stylesheet>` 等) が将来
    // Author として届く経路と混同しないため。
    for extra in options.extra_stylesheets {
        let expanded = expand_stylesheet_imports_with_budget(
            extra,
            effective_base.as_ref(),
            None,
            options.network,
            &mut doc.warnings,
            &mut import_budget,
        );
        doc.dom
            .add_stylesheet(Cow::Owned(expanded), StylesheetKind::User);
    }

    // <link rel="stylesheet" href="..."> を検出し、ParseOptions::network
    // 経由で fetch、CSS text を Author stylesheet source として
    // doc.stylesheet_sources に統合する。
    fetch_external_stylesheets(&mut doc, options, &mut import_budget);

    Ok(doc)
}

/// `<head>` 内の stylesheet-bearing elements を document order で処理し、成功した
/// external CSS を `doc.stylesheet_sources` に Author origin として追加する。
/// Inline `<style>` と `<link>` は同じ `stylesheet_sources` bucket 内で元の
/// head order に並ぶ。`raikiri` umbrella の `build_cascaded` はこの Vec を
/// 丸ごと Author として消費するため、umbrella 側の API 変更は不要である。
///
/// href の検出は `sink::collect_head_stylesheet_sources` (`finish()` 後の
/// `Document` を読むだけの純粋関数) が担う。実 fetch は `TreeSink::finish()`
/// の外で行うため、Sink 実装は I/O を持たない。`options.network` が `None`
/// の場合は external stylesheet を処理しない。fetch 失敗は fatal にせず
/// `doc.warnings` に記録して parse 全体を継続する。
///
/// # 既知の scope 制限
///
/// - `Document::stylesheets()` (UA CSS / extra stylesheets) は既存の別 bucket
///   なので、head stylesheet より先に cascade される。extra stylesheet 内の
///   imports はこの post-processing pass より前に展開される。
/// - **`disabled` / `media` / `crossorigin` / `integrity`**:
///   `sink::collect_external_stylesheet_hrefs` doc 参照。
/// - **`<base>` の探索範囲と href の扱い**:
///   `sink::find_document_base_href` doc 参照。frozen base URL algorithm の
///   Document-level security policy は本 crate に相当する概念がないため未実装。
/// - **`<base>` と `<link>` の相対順序**: 全 parse 完了後に一括 fetch するため、
///   links before a later `<base>` also use the final effective base URL.
/// - **encoding**: HTML body の parse と同様 UTF-8 前提
///   (`String::from_utf8_lossy`)。非 UTF-8 CSS の decoding は将来対応する。
///
fn fetch_external_stylesheets(
    doc: &mut UncascadedDocument,
    options: &ParseOptions<'_>,
    import_budget: &mut ImportBudget,
) {
    let Some(network) = options.network else {
        return;
    };

    // HTML Standard §4.2.7 "The base element" / "document base URL": a
    // <base href> in <head>, if present, overrides options.base_url as the
    // base for resolving <link href> and inline stylesheet imports.
    let effective_base = effective_document_base_url(doc, options.base_url.as_ref());

    // `finish()` has already projected inline sources into this public Vec.
    // Rebuild it in the order of the original head elements so a fetched link
    // does not silently move after every inline style.
    let head_sources = crate::sink::collect_head_stylesheet_sources(&doc.dom);
    let mut inline_sources = std::mem::take(&mut doc.stylesheet_sources).into_iter();
    if head_sources.is_empty() {
        // Preserve the generic `parse_with_sink` contract for a consumer sink
        // that supplies stylesheet_sources without a normal HTML `<head>`.
        doc.stylesheet_sources = inline_sources
            .map(|css| {
                expand_stylesheet_imports_with_budget(
                    &css,
                    effective_base.as_ref(),
                    None,
                    Some(network),
                    &mut doc.warnings,
                    import_budget,
                )
            })
            .collect();
        return;
    }
    let mut ordered_sources = Vec::new();
    for source in head_sources {
        match source {
            crate::sink::HeadStylesheetSource::Inline { .. } => {
                if let Some(css) = inline_sources.next() {
                    let expanded = expand_stylesheet_imports_with_budget(
                        &css,
                        effective_base.as_ref(),
                        None,
                        Some(network),
                        &mut doc.warnings,
                        import_budget,
                    );
                    ordered_sources.push(expanded);
                }
            }
            crate::sink::HeadStylesheetSource::External { node_id, href } => {
                let Some(url) = resolve_url(&href, effective_base.as_ref()) else {
                    // Relative URL without a base, or otherwise invalid href:
                    // there is no request to report and the link contributes no
                    // stylesheet source.
                    continue;
                };

                let request = Request {
                    url: url.clone(),
                    method: Method::Get,
                    content_type: None,
                    headers: Vec::new(),
                    body: Body::Empty,
                    signal: None,
                    kind: ResourceKind::ExternalStylesheet,
                };

                match network.fetch(request) {
                    Ok(fetched) => {
                        // A successful empty response contributes no source.
                        let css = String::from_utf8_lossy(&fetched.bytes).into_owned();
                        if !css.is_empty() {
                            let expanded = expand_stylesheet_imports_with_budget(
                                &css,
                                Some(&fetched.final_url),
                                Some(&fetched.final_url),
                                Some(network),
                                &mut doc.warnings,
                                import_budget,
                            );
                            ordered_sources.push(expanded);
                        }
                    }
                    Err(NetworkError::PolicyViolation(violation)) => {
                        // ResourcePolicy が拒否した (SandboxedNetProvider 等、既存
                        // sandboxing 契約側の判定) — 専用 warning variant が既にある
                        // のでそれを使う。URL と provider-controlled details は
                        // warning に漏らさない。
                        doc.warnings.push(RenderWarning {
                            kind: WarningKind::PolicyWarning {
                                violation: sanitize_policy_violation(violation),
                            },
                            node_id: Some(node_id),
                            details: "<link rel=stylesheet>: fetch denied by resource policy"
                                .to_owned(),
                        });
                    }
                    Err(err) => {
                        // All other errors are non-fatal. Keep a safe,
                        // structured summary instead of formatting arbitrary
                        // provider-controlled error text into the warning.
                        let safe_url = redacted_url(&url);
                        let summary = network_error_summary(&err);
                        doc.warnings.push(RenderWarning {
                            kind: WarningKind::NetworkFallback {
                                url: safe_url.clone(),
                            },
                            node_id: Some(node_id),
                            details: format!(
                                "<link rel=stylesheet>: fetch failed for {safe_url}: {summary}"
                            ),
                        });
                    }
                }
            }
        }
    }
    // Defensive: preserve any inline projection that did not have a matching
    // collector entry if the sink projection changes in the future.
    ordered_sources.extend(inline_sources.map(|css| {
        expand_stylesheet_imports_with_budget(
            &css,
            effective_base.as_ref(),
            None,
            Some(network),
            &mut doc.warnings,
            import_budget,
        )
    }));
    doc.stylesheet_sources = ordered_sources;
}

/// Resolve the document's effective base URL for stylesheet and link fetches.
///
/// A valid `<base>` in the document is resolved against the caller-provided
/// fallback. Invalid, `data:`, and `javascript:` base URLs fall back to the
/// caller-provided URL, matching the existing link-fetch behavior.
fn effective_document_base_url(doc: &UncascadedDocument, fallback: Option<&Url>) -> Option<Url> {
    match crate::sink::find_document_base_href(&doc.dom) {
        Some(base_href) => resolve_url(&base_href, fallback)
            .filter(|url| !matches!(url.scheme(), "data" | "javascript"))
            .or_else(|| fallback.cloned()),
        None => fallback.cloned(),
    }
}

/// Resolve `href` against `base`. Absolute URLs do not need a base; relative
/// URLs are rejected when no base is available.
fn resolve_url(href: &str, base: Option<&Url>) -> Option<Url> {
    match base {
        Some(base) => base.join(href).ok(),
        None => Url::parse(href).ok(),
    }
}
