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
/// を返す (M1 spike scope、encoding_rs 導入は M2+ で予定)。
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
    parse_with_sink(input, RaikiriTreeSink::new(), options)
}

/// Consumer-supplied sink 経由で parse する。Consumer wrapper は
/// `type Output = UncascadedDocument` を宣言し、`finish(self)` で inner
/// sink の finish 結果を bubble させる契約。
///
/// M1.4a (m1.22) 以降、parse 完了時に既定 UA CSS + `options.extra_stylesheets`
/// を [`raikiri_dom::Document::add_stylesheet`] 経由で Document 状態に注入する
/// (spec §M1.4a、UA=UserAgent/extra=Author kind)。M2 (raikiri-spike-5z86.6)
/// 以降、続けて `<head>` 内 `<link rel="stylesheet">` を `options.network` /
/// `options.base_url` 経由で fetch し、成功分を `UncascadedDocument.stylesheet_sources`
/// に Author として追加する (`fetch_external_stylesheets` doc 参照)。
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

    // spec §M1.4a: 既定 UA CSS を Document に注入 (raikiri-spike-m1.22)
    doc.dom.add_stylesheet(
        Cow::Borrowed(crate::ua::MINIMAL_UA_CSS),
        StylesheetKind::UserAgent,
    );

    // Consumer 提供の extra_stylesheets を User origin として追加 (spec §M1
    // ParseOptions::extra_stylesheets の実 consume 経路)。bd raikiri-spike-d7h3
    // で StylesheetKind::Author retag から独立 StylesheetKind::User へ移行 —
    // real author-origin stylesheet (`<link rel=stylesheet>` 等) が将来
    // Author として届く経路と混同しないため。
    for extra in options.extra_stylesheets {
        doc.dom
            .add_stylesheet(Cow::Owned((*extra).to_string()), StylesheetKind::User);
    }

    // spec §M2 (raikiri-spike-5z86.6): <link rel="stylesheet" href="..."> を
    // 検出し、ParseOptions::network 経由で fetch、CSS text を Author
    // stylesheet source として doc.stylesheet_sources に統合する。
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
/// # 既知の scope 制限 (raikiri-spike-5z86.6)
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
/// - **encoding**: HTML body の parse と同様 UTF-8 前提
///   (`String::from_utf8_lossy`)、非 UTF-8 CSS は文字化けする。encoding_rs
///   導入は本 crate の `parse()` 全体で M2+ に defer 済み (`parse()` doc 参照)。
fn fetch_external_stylesheets(doc: &mut UncascadedDocument, options: &ParseOptions<'_>) {
    let Some(network) = options.network else {
        return;
    };

    for (node_id, href) in crate::sink::collect_external_stylesheet_hrefs(&doc.dom) {
        let Some(url) = resolve_stylesheet_url(&href, options.base_url.as_ref()) else {
            // 解決不能な href (相対 URL なのに base_url 未提供、または href
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
                // 続行する」という結果は同じなので NetworkFallback に統一する
                // (raikiri-traits に新 WarningKind variant を追加すると
                // wall/traits 対象になるため、既存 variant の意味論を「fetch
                // が失敗しこの資源を諦めた」という broad な読みで再利用する
                // 判断— 詳細メッセージは details に載せて可観測性を保つ)。
                //
                // Semantic gap (reviewer-spec finding, bd raikiri-spike-5z86.8
                // tracks a dedicated fix): `NetworkFallback`'s doc comment
                // ("Consumer の network が fallback を返した") reads most
                // naturally as `ResolverFallback`'s sibling — "the fetch
                // succeeded with an explicit substitute", i.e. degraded-but-
                // present content. This use is different: the fetch returned
                // `Err`, and no CSS is applied *at all* for this `<link>` —
                // a total skip, not a substitution. Warning consumers must
                // not infer "some (possibly stale/placeholder) stylesheet
                // content was applied" from this variant here the way they
                // reasonably could for a true fallback-substitution use of
                // `NetworkFallback` elsewhere; the correct reading for THIS
                // call site is "nothing was applied for this stylesheet".
                doc.warnings.push(RenderWarning {
                    kind: WarningKind::NetworkFallback { url },
                    node_id: Some(node_id),
                    details: format!("<link rel=stylesheet href={href:?}>: fetch failed: {err}"),
                });
            }
        }
    }
}

/// `href` を `base_url` に対して resolve する。`href` が既に absolute URL な
/// ら `base_url` は無視される (`Url::join` の標準挙動)。`base_url` が無い
/// 場合は `href` 自体が absolute な場合のみ成功する。
fn resolve_stylesheet_url(href: &str, base_url: Option<&Url>) -> Option<Url> {
    match base_url {
        Some(base) => base.join(href).ok(),
        None => Url::parse(href).ok(),
    }
}
