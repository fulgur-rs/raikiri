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
/// (UA=UserAgent/extra=Author kind)。続けて `<head>` 内
/// `<link rel="stylesheet">` を `options.network` / `options.base_url`
/// 経由で fetch し、成功分を `UncascadedDocument.stylesheet_sources` に
/// Author として追加する (`fetch_external_stylesheets` doc 参照)。
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

    // Consumer 提供の extra_stylesheets を User origin として追加
    // (ParseOptions::extra_stylesheets の実 consume 経路)。
    // StylesheetKind::Author retag から独立 StylesheetKind::User へ移行済み —
    // real author-origin stylesheet (`<link rel=stylesheet>` 等) が将来
    // Author として届く経路と混同しないため。
    for extra in options.extra_stylesheets {
        doc.dom
            .add_stylesheet(Cow::Owned((*extra).to_string()), StylesheetKind::User);
    }

    // <link rel="stylesheet" href="..."> を検出し、ParseOptions::network
    // 経由で fetch、CSS text を Author stylesheet source として
    // doc.stylesheet_sources に統合する。
    fetch_external_stylesheets(&mut doc, options);

    Ok(doc)
}

/// `<head>` 内 `<link rel="stylesheet">` を fetch し、成功した CSS text を
/// `doc.stylesheet_sources` に追加する (Author origin、既存の `<style>`
/// 抽出結果と同じ bucket — raikiri umbrella crate の `build_cascaded` が
/// `stylesheet_sources` を丸ごと Author として消費するので、ここに追加する
/// だけで umbrella 側は無変更のまま cascade に統合される)。
///
/// href の検出自体は `sink::collect_external_stylesheet_hrefs` (`finish()` 後
/// の `Document` を読むだけの純粋関数、副作用なし) が担う。実 fetch は
/// `TreeSink::finish()` の外、ここで行う — Sink 実装は I/O を持たない契約を
/// 保つ必要がある (sink 実装が観測可能な副作用を追加すると wall/sink 対象)。
///
/// `options.network` が `None` の場合 (Consumer が network capability を渡し
/// ていない) は何もしない — 外部 stylesheet 機能は opt-in。fetch 失敗
/// (network error / policy violation) は fatal にせず `doc.warnings` に記録
/// して parse 全体は継続する (html5ever の forgiving-parsing の精神、および
/// 既存の `HtmlParseError` warning 降格パターンに合わせる)。
///
/// # 既知の scope 制限
///
/// - **順序**: `doc.stylesheet_sources` には既に (`<style>` 抽出由来の) head
///   内 `<style>` テキストが document order で入っている。ここで fetch した
///   外部 stylesheet はその**後ろ**に追記するため、`<style>` と `<link>` が
///   同一 `<head>` 内で入り交じる文書では真の document-order interleave に
///   ならない (cascade の same-specificity tie-break にのみ影響。既存の
///   `Document::stylesheets()` (UA CSS / extra_stylesheets) と
///   `stylesheet_sources` (head `<style>`) の 2-bucket 方式も同様に文書上の
///   真の出現順とは無関係に前者が必ず先行する — 本 task 固有の妥協ではなく
///   既存 architecture の延長)。
/// - **`disabled` / `media` / `crossorigin` / `integrity`**:
///   `sink::collect_external_stylesheet_hrefs` doc 参照。
/// - **`<base>` の探索範囲と href="" の扱い**:
///   `sink::find_document_base_href` doc 参照 (head-only DFS、非空 href を
///   持つ最初の `<base>` を採用)。frozen base URL algorithm の "Is base
///   allowed for Document?" チェック (Document 単位の security policy) は
///   本 crate に相当する概念が無いため未実装 — `data:` / `javascript:`
///   scheme の除外のみ実装する。
/// - **`<base>` と `<link>` の相対順序**: HTML Standard の実際の処理モデル
///   では、`<link>` の外部 resource fetch はパーサがその `<link>` を挿入
///   した時点の document base URL に対して行われる — 同じ `<head>` 内で
///   `<link>` が `<base>` より**前**にあれば、その `<link>` は override 前の
///   `options.base_url` で解決されるべきで、後から出現する `<base>` は遡って
///   適用されない。本実装は全 parse 完了後の単一 post-processing pass で
///   `<head>` 内の全 `<link>` を一括 fetch するため、この出現順の違いを
///   区別できず、`<head>` 内のどの `<link>` にも (前後を問わず) 同じ
///   effective base を一律適用する (上記「順序」bullet と同じ single-pass
///   architecture に起因する制約)。
/// - **encoding**: HTML body の parse と同様 UTF-8 前提
///   (`String::from_utf8_lossy`)、非 UTF-8 CSS は文字化けする。encoding_rs
///   導入は本 crate の `parse()` 全体で将来に defer 済み (`parse()` doc 参照)。
fn fetch_external_stylesheets(doc: &mut UncascadedDocument, options: &ParseOptions<'_>) {
    let Some(network) = options.network else {
        return;
    };

    // HTML Standard §4.2.7 "The base element" / "document base URL": a
    // <base href> in <head>, if present, overrides options.base_url as the
    // base for resolving <link href>. The <base>'s own href can itself be
    // relative, so it is resolved against options.base_url first
    // (`find_document_base_href` doc comment covers the head-only search
    // scope and the empty-href edge case).
    //
    // Per the base element's "frozen base URL" algorithm, the resolved URL
    // is discarded in favor of the fallback base URL (options.base_url,
    // unchanged) when it: (a) fails to parse — e.g. a relative <base href>
    // with no options.base_url to resolve against; or (b) has a `data:` or
    // `javascript:` scheme. (The algorithm's third exclusion, "Is base
    // allowed for Document?", is a Document-level security policy concept
    // this crate has no equivalent of, and is not implemented here.)
    //
    // This applies uniformly to every <link> in <head>, regardless of
    // whether it appears before or after the <base> in source order (this
    // function's doc comment, "<base> と <link> の相対順序" bullet, covers
    // why: a single post-parse fetch pass can't distinguish "before" from
    // "after").
    let effective_base: Option<Url> = match crate::sink::find_document_base_href(&doc.dom) {
        Some(base_href) => resolve_url(&base_href, options.base_url.as_ref())
            .filter(|url| !matches!(url.scheme(), "data" | "javascript"))
            .or_else(|| options.base_url.clone()),
        None => options.base_url.clone(),
    };

    for (node_id, href) in crate::sink::collect_external_stylesheet_hrefs(&doc.dom) {
        let Some(url) = resolve_url(&href, effective_base.as_ref()) else {
            // 解決不能な href (相対 URL なのに base 未提供、または href
            // 自体が invalid) — fetch しようがないので silent skip。
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
                // extract_inline_stylesheets と同じ empty-body guard: 200-with-
                // empty-body な応答を無意味な空 stylesheet として追加しない。
                let css = String::from_utf8_lossy(&fetched.bytes).into_owned();
                if !css.is_empty() {
                    doc.stylesheet_sources.push(css);
                }
            }
            Err(NetworkError::PolicyViolation(violation)) => {
                // ResourcePolicy が拒否した (SandboxedNetProvider 等、既存
                // sandboxing 契約側の判定) — 専用 warning variant が既にある
                // のでそれを使う。
                doc.warnings.push(RenderWarning {
                    kind: WarningKind::PolicyWarning { violation },
                    node_id: Some(node_id),
                    details: format!(
                        "<link rel=stylesheet href={href:?}>: fetch denied by resource policy"
                    ),
                });
            }
            Err(err) => {
                // Aborted / Io / Http / Other: どれも「この stylesheet は諦めて
                // 続行する」という結果は同じなので NetworkFallback に統一する。
                // これは content が一切適用されない Err-disposition の使用
                // であり、`WarningKind::NetworkFallback` のドキュメントが
                // 明示的に扱う 2 つの disposition のうちの一方 (詳細は
                // raikiri-traits 側の doc 参照)。個々の `NetworkError` variant
                // の区別自体は失わず、details に元の error の Display 出力を
                // 埋め込んで可観測性を保つ。
                doc.warnings.push(RenderWarning {
                    kind: WarningKind::NetworkFallback { url },
                    node_id: Some(node_id),
                    details: format!("<link rel=stylesheet href={href:?}>: fetch failed: {err}"),
                });
            }
        }
    }
}

/// `href` を `base` に対して resolve する。`href` が既に absolute URL なら
/// `base` は無視される (`Url::join` の標準挙動)。`base` が無い場合は `href`
/// 自体が absolute な場合のみ成功する。
///
/// 呼び出し元は 2 つ: `<link rel=stylesheet href>` の解決 (`base` は
/// `<base>` element を織り込んだ effective base)、および `<base href>` 自身
/// の解決 (`base` は `options.base_url` そのもの — spec 上 `<base>` の href
/// は常に document の fallback base URL に対して解決される)。
fn resolve_url(href: &str, base: Option<&Url>) -> Option<Url> {
    match base {
        Some(base) => base.join(href).ok(),
        None => Url::parse(href).ok(),
    }
}
