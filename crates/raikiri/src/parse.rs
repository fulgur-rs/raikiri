//! `parse_html`: HTML byte stream から cascade 済 [`HtmlDocument`] を生成する
//! orchestrator。
//!
//! spec §L1060 の pub API 相当。内部 pipeline は
//! [`raikiri_html::parse`] → rule-tree build and first-page cascade → assemble。
//! 現状 cascade は
//! 常に `Ok` を返すため、`RenderError::Parse` のみが bubble する。
//!
//! # Input byte cap
//!
//! `parse_html_with_limits` は [`RenderLimits::max_input_bytes`] を read して
//! bounded read を行い、SEC-HIGH の `parse_html` unbounded-read DoS (attacker
//! が任意サイズの HTML を送り込み OOM を誘発)
//! を close する。実装は `Read::take(cap + 1)` + `read_to_end` の "probe" pattern:
//! read 完了後 `buf.len() > cap` を検出したら
//! `RenderError::LimitExceeded { kind: LimitKind::InputBytes, .. }` を返す
//! (html5ever は truncated input を silently accept するため、単純 `take`
//! では検出できない)。
//!
//! 当初は hard-coded `INPUT_BYTES_CAP` const と `LimitKind::AggregateBytes`
//! re-use の stopgap で SEC-HIGH を close していた。その後
//! `RenderLimits::max_input_bytes: Option<u64>` (default `Some(32 * 1024 * 1024)`)
//! および `LimitKind::InputBytes` に昇格した (wall/traits crossing)。
//! default 値は旧 stopgap と同一のため、
//! `RenderLimits::default()` を渡す consumer は behavior 不変。cap を調整したい
//! consumer は [`RenderLimitsBuilder::max_input_bytes`](raikiri_traits::RenderLimitsBuilder::max_input_bytes)
//! (または field への直接代入)、無効化したい consumer は `max_input_bytes = None`
//! を設定する (**cap 無効化は SEC-HIGH の DoS を再暴露する** — field doc
//! の Security note 参照)。

use std::io::Read;

use raikiri_html::{ParseOptions, RaikiriTreeSink, effective_document_base_url, parse_with_sink};
use raikiri_traits::{LimitKind, ParseError, RenderError, RenderLimits};

use crate::{HtmlDocument, PageContextQuery, build_rule_tree};

/// HTML byte stream を parse し、cascade まで完了した [`HtmlDocument`] を返す。
///
/// 内部で [`parse_html_with_limits`] に [`RenderLimits::default()`] を渡す
/// thin wrapper。Consumer が既存 `parse_html` を呼び出しても default 32 MiB
/// input cap (`RenderLimits::default().max_input_bytes = Some(32 * 1024 * 1024)`)
/// は必ず enforce される (SEC-HIGH close 済み、advertised path が bounded、
/// fail-closed 原則)。
///
/// # Errors
///
/// - `RenderError::Parse(ParseError::Io)`: `input` の read が Err を返した
/// - `RenderError::Parse(ParseError::Encoding)`: 入力が valid UTF-8 でない
/// - `RenderError::Parse(ParseError::*)`: html5ever が Parse error を返した
/// - `RenderError::LimitExceeded { kind: LimitKind::InputBytes, .. }`:
///   input byte 数が [`RenderLimits::max_input_bytes`] を超えた
///
/// 現状 cascade は infallible (`.expect` で unwrap)。
///
/// # Example
///
/// ```
/// use raikiri::{parse_html, ParseOptions};
///
/// let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
/// let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse");
/// assert!(!doc.cascade().computed.is_empty());
/// ```
#[allow(clippy::result_large_err)]
pub fn parse_html<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
) -> Result<HtmlDocument, RenderError> {
    parse_html_with_limits(input, options, RenderLimits::default())
}

/// `parse_html` と同じ pipeline を、`RenderLimits` を明示的に受け取る形で
/// 提供する variant。[`RenderLimits::max_input_bytes`] を consult するよう昇格:
///
/// * `Some(cap)` の場合、input read を `cap + 1` bytes で bounded probe
///   し、`cap` 超過なら [`RenderError::LimitExceeded`] で早期返却
/// * `None` の場合、input を無制限に read する (Consumer が明示的に cap を
///   無効化した場合のみ、fail-closed default は 32 MiB)
///
/// [`RenderLimits::max_parse_warnings`] も consult される: `raikiri_html`
/// の `RaikiriTreeSink` に直接渡され、html5ever が報告する非致命 parse
/// error を warning として記録する件数を cap する (詳細は field doc 参照、
/// input byte cap と異なりこちらは早期 return しない — 超過分は黙って
/// drop される代わりに synthetic な 1 件の warning が追加される)。
///
/// その他の `limits.*` field (`max_dom_nodes` / `max_aggregate_bytes` / etc.)
/// は現時点で `parse_html_with_limits` 内では **consult されない** — これらは
/// downstream renderer / cascade layers 用に予約されている。
///
/// # Implementation
///
/// `Some(cap)` 経路: `input.by_ref().take(cap + 1).read_to_end(&mut buf)` で
/// bounded read。`buf.len() > cap` なら input が cap を超えたと確定できる
/// (Read::take だけでは cap 到達時 silently truncate される + html5ever も
/// truncated input を無警告で parse するため、この "+1 probe" が無いと cap
/// 到達判定ができない)。
///
/// `None` 経路: `input.read_to_end(&mut buf)` で unbounded read。
///
/// # Errors
///
/// - `RenderError::Parse(ParseError::Io)`: `input` の read が Err を返した
/// - `RenderError::Parse(ParseError::Encoding)`: 入力が valid UTF-8 でない
/// - `RenderError::Parse(ParseError::*)`: html5ever が Parse error を返した
/// - `RenderError::LimitExceeded { kind: LimitKind::InputBytes, limit, actual }`:
///   input byte 数が `limits.max_input_bytes.unwrap()` を超えた。`limit` =
///   設定 cap、`actual` は cap を超えたことのみ確定 (真の input size は cap
///   超過 detection の都合で不明、"cap を超えたことは確実" と読む)
#[allow(clippy::result_large_err)]
pub fn parse_html_with_limits<R: Read>(
    mut input: R,
    options: &ParseOptions<'_>,
    limits: RenderLimits,
) -> Result<HtmlDocument, RenderError> {
    let mut buf: Vec<u8> = Vec::new();
    match limits.max_input_bytes {
        Some(cap) => {
            // "+1 probe" pattern: cap + 1 byte 読めてしまったら cap 超過確定。
            // saturating_add で cap == u64::MAX の overflow を guard。
            let probe_cap = cap.saturating_add(1);
            input
                .by_ref()
                .take(probe_cap)
                .read_to_end(&mut buf)
                .map_err(|e| RenderError::Parse(ParseError::Io(e)))?;

            if (buf.len() as u64) > cap {
                return Err(RenderError::LimitExceeded {
                    kind: LimitKind::InputBytes,
                    limit: cap,
                    actual: buf.len() as u64,
                });
            }
        }
        None => {
            // Consumer が明示的に cap 無効化 (fail-closed default は 32 MiB、
            // ここに来るのは opt-out した場合のみ)。
            input
                .read_to_end(&mut buf)
                .map_err(|e| RenderError::Parse(ParseError::Io(e)))?;
        }
    }

    // Under cap: materialized slice を raikiri_html::parse_with_sink に渡す
    // (raikiri_html::parse の thin wrapper 経路だと sink が
    // `RaikiriTreeSink::default()` 固定になり `limits.max_parse_warnings` を
    // consult できないため、ここでは sink を明示的に construct する)。
    // parse_with_sink は内部で `read_to_end` するため、`&[u8]` を渡すと 1 回の
    // memcpy で済む (bytes 二重 alloc は避けられないが、cap 分の memory が上限)。
    let sink = RaikiriTreeSink::new(limits.max_parse_warnings);
    let uncascaded = parse_with_sink(buf.as_slice(), sink, options).map_err(RenderError::Parse)?;
    let effective_base_url = effective_document_base_url(&uncascaded, options.base_url.as_ref());
    let mut first_page = PageContextQuery::default();
    first_page.is_first = true;
    first_page.is_right = true;
    let rule_tree = build_rule_tree(&uncascaded);
    let font_faces = rule_tree.font_faces().clone();
    let cascade = raikiri_style::cascade_with_media_context_for_page(
        &uncascaded.dom,
        &rule_tree,
        &raikiri_style::MediaContext::default(),
        &first_page,
    )
    .expect("cascade は常に Ok のはず");
    Ok(HtmlDocument {
        uncascaded,
        cascade,
        font_faces,
        effective_base_url,
    })
}
