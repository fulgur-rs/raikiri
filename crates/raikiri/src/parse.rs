//! `parse_html`: HTML byte stream から cascade 済 [`HtmlDocument`] を生成する
//! orchestrator (raikiri-spike-m1.11)。
//!
//! spec §L1060 の pub API 相当。内部 pipeline は
//! `raikiri_html::parse` → [`build_cascaded`] → assemble。M1 では cascade は
//! 常に `Ok` を返すため、`RenderError::Parse` のみが bubble する。
//!
//! # Input byte cap (raikiri-spike-d9y.3, Sprint 9 Wave 3 — Option B stopgap)
//!
//! `parse_html_with_limits` は **hard-coded 32 MiB input cap** で bounded read
//! を行い、SEC-HIGH の `parse_html` unbounded-read DoS (attacker が任意サイズの
//! HTML を送り込み OOM を誘発) を close する。実装は `Read::take(cap + 1)` +
//! `read_to_end` の "probe" pattern: read 完了後 `buf.len() > cap` を検出したら
//! `RenderError::LimitExceeded` を返す (html5ever は truncated input を
//! silently accept するため、単純 `take` では検出できない)。
//!
//! Option B stopgap の semantic mismatch: 現状は `LimitKind::AggregateBytes` を
//! re-use しているが、AggregateBytes は本来 approximate memory footprint 用途で
//! 意味が広い。input-byte 専用の `LimitKind::InputBytes` variant + tunable
//! `RenderLimits::max_input_bytes` field は follow-up bd task
//! `raikiri-spike-4kw` (wall/traits crossing、human approve 必須) が
//! Option A として担う。それまで本 stopgap で SEC-HIGH を実質 close する。

use std::io::Read;

use raikiri_html::ParseOptions;
use raikiri_traits::{LimitKind, ParseError, RenderError, RenderLimits};

use crate::{HtmlDocument, build_cascaded};

/// `parse_html_with_limits` が enforce する input byte cap (32 MiB)。
///
/// TODO(follow-up bd raikiri-spike-4kw): promote hard-coded `INPUT_BYTES_CAP`
/// to `raikiri_traits::RenderLimits::max_input_bytes` when wall/traits approve
/// is available. Until then this const is the sole knob and none of the
/// current `RenderLimits` fields (`max_aggregate_bytes` / `max_dom_nodes` /
/// `max_render_seconds` / …) are consulted by parse — those are reserved for
/// downstream renderer / cascade layers.
const INPUT_BYTES_CAP: u64 = 32 * 1024 * 1024;

/// HTML byte stream を parse し、cascade まで完了した [`HtmlDocument`] を返す。
///
/// 内部で [`parse_html_with_limits`] に [`RenderLimits::default()`] を渡す
/// thin wrapper。Consumer が既存 `parse_html` を呼び出しても hard-coded
/// 32 MiB input cap は必ず enforce される (SEC-HIGH d9y.3 close、advertised
/// path が bounded、fail-closed 原則)。
///
/// # Errors
///
/// - `RenderError::Parse(ParseError::Io)`: `input` の read が Err を返した
/// - `RenderError::Parse(ParseError::Encoding)`: 入力が valid UTF-8 でない
/// - `RenderError::Parse(ParseError::*)`: html5ever が Parse error を返した
/// - `RenderError::LimitExceeded { kind: LimitKind::AggregateBytes, .. }`:
///   input byte 数が [`INPUT_BYTES_CAP`] を超えた (Option B stopgap: semantic
///   上は input-byte 専用の variant が望ましいが、現状は AggregateBytes を
///   re-use。follow-up `raikiri-spike-4kw` 参照)
///
/// M1 では cascade は infallible (m1.23 契約、`.expect` で unwrap)。
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
/// 提供する variant。Option B stopgap では `limits` の各 field は現時点で
/// **consult されず** (`max_dom_nodes` / `max_aggregate_bytes` / etc.)、
/// hard-coded `INPUT_BYTES_CAP` (32 MiB) のみが input read で enforce される。
///
/// Signature は将来 Option A (follow-up `raikiri-spike-4kw`) で
/// `limits.max_input_bytes` を read するよう switch する為に予め placeholder
/// として receive しておく (source-compat: 呼び出し側の call-site は
/// Option A への upgrade 時に修正不要)。
///
/// # Semantic note
///
/// Input byte cap 超過は [`LimitKind::AggregateBytes`] として報告される。
/// これは Option B stopgap の trade-off で、Option A 完了後は
/// `LimitKind::InputBytes` に差し替わる予定 (follow-up bd task)。
///
/// # Implementation
///
/// `input.take(INPUT_BYTES_CAP + 1).read_to_end(&mut buf)` で bounded read。
/// `buf.len() > INPUT_BYTES_CAP` なら input が cap を超えたと確定できるので
/// `RenderError::LimitExceeded` を返す。この "+1 probe" が無いと、
/// `Read::take(cap)` だけでは cap 到達時 silently truncate される (html5ever
/// も truncated input を無警告で parse) ため cap 到達判定ができない。
///
/// # Errors
///
/// - `RenderError::Parse(ParseError::Io)`: `input` の read が Err を返した
/// - `RenderError::Parse(ParseError::Encoding)`: 入力が valid UTF-8 でない
/// - `RenderError::Parse(ParseError::*)`: html5ever が Parse error を返した
/// - `RenderError::LimitExceeded { kind: LimitKind::AggregateBytes, limit, actual }`:
///   input byte 数が [`INPUT_BYTES_CAP`] を超えた。`limit` = `INPUT_BYTES_CAP`、
///   `actual` = `INPUT_BYTES_CAP + 1` (真の input size は cap 超過 detection の
///   都合で不明、"cap を超えたことは確実" と読む)
#[allow(clippy::result_large_err)]
pub fn parse_html_with_limits<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
    _limits: RenderLimits,
) -> Result<HtmlDocument, RenderError> {
    // "+1 probe" pattern: cap + 1 byte 読めてしまったら cap 超過確定。
    // usize/u64 変換は saturating (32 bit target で INPUT_BYTES_CAP が
    // usize::MAX を超えないようにするため)。
    let probe_cap = INPUT_BYTES_CAP.saturating_add(1);
    let mut buf: Vec<u8> = Vec::new();
    input
        .take(probe_cap)
        .read_to_end(&mut buf)
        .map_err(|e| RenderError::Parse(ParseError::Io(e)))?;

    if (buf.len() as u64) > INPUT_BYTES_CAP {
        return Err(RenderError::LimitExceeded {
            kind: LimitKind::AggregateBytes,
            limit: INPUT_BYTES_CAP,
            actual: buf.len() as u64,
        });
    }

    // Under cap: 既存 raikiri_html::parse に materialized slice で渡す。
    // raikiri_html::parse は内部で `read_to_end` するため、`&[u8]` を渡すと
    // 1 回の memcpy で済む (bytes 二重 alloc は避けられないが、cap 分の
    // 32 MiB が memory 上限)。
    let uncascaded = raikiri_html::parse(buf.as_slice(), options).map_err(RenderError::Parse)?;
    let cascade = build_cascaded(&uncascaded);
    Ok(HtmlDocument {
        uncascaded,
        cascade,
    })
}
