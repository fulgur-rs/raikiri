//! `parse_html`: HTML byte stream から cascade 済 [`HtmlDocument`] を生成する
//! orchestrator (raikiri-spike-m1.11)。
//!
//! spec §L1060 の pub API 相当。内部 pipeline は
//! `raikiri_html::parse` → [`build_cascaded`] → assemble。M1 では cascade は
//! 常に `Ok` を返すため、`RenderError::Parse` のみが bubble する。
//!
//! # Input byte cap (Sprint 10 Option A — bd raikiri-spike-4kw)
//!
//! `parse_html_with_limits` は [`RenderLimits::max_input_bytes`] を read して
//! bounded read を行い、SEC-HIGH の `parse_html` unbounded-read DoS (attacker
//! が任意サイズの HTML を送り込み OOM を誘発、Sprint 9 Wave 3 bd raikiri-spike-d9y.3)
//! を close する。実装は `Read::take(cap + 1)` + `read_to_end` の "probe" pattern:
//! read 完了後 `buf.len() > cap` を検出したら
//! `RenderError::LimitExceeded { kind: LimitKind::InputBytes, .. }` を返す
//! (html5ever は truncated input を silently accept するため、単純 `take`
//! では検出できない)。
//!
//! Wave 3 では hard-coded `INPUT_BYTES_CAP` const と `LimitKind::AggregateBytes`
//! re-use の Option B stopgap で SEC-HIGH を close していた。Sprint 10 で
//! `RenderLimits::max_input_bytes: Option<u64>` (default `Some(32 * 1024 * 1024)`)
//! および `LimitKind::InputBytes` に昇格 (Option A、wall/traits crossing、
//! bd raikiri-spike-sve decision)。default 値は Wave 3 stopgap と同一のため、
//! `RenderLimits::default()` を渡す consumer は behavior 不変。cap を無効化
//! したい consumer は `max_input_bytes = None` を、より大きな cap を設定したい
//! consumer は [`RenderLimits::with_max_input_bytes`] を利用する。

use std::io::Read;

use raikiri_html::ParseOptions;
use raikiri_traits::{LimitKind, ParseError, RenderError, RenderLimits};

use crate::{HtmlDocument, build_cascaded};

/// HTML byte stream を parse し、cascade まで完了した [`HtmlDocument`] を返す。
///
/// 内部で [`parse_html_with_limits`] に [`RenderLimits::default()`] を渡す
/// thin wrapper。Consumer が既存 `parse_html` を呼び出しても default 32 MiB
/// input cap (`RenderLimits::default().max_input_bytes = Some(32 * 1024 * 1024)`)
/// は必ず enforce される (SEC-HIGH d9y.3 close、advertised path が bounded、
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
/// 提供する variant。Sprint 10 Option A (bd raikiri-spike-4kw) で
/// [`RenderLimits::max_input_bytes`] を consult するよう昇格:
///
/// * `Some(cap)` の場合、input read を `cap + 1` bytes で bounded probe
///   し、`cap` 超過なら [`RenderError::LimitExceeded`] で早期返却
/// * `None` の場合、input を無制限に read する (Consumer が明示的に cap を
///   無効化した場合のみ、fail-closed default は 32 MiB)
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

    // Under cap: 既存 raikiri_html::parse に materialized slice で渡す。
    // raikiri_html::parse は内部で `read_to_end` するため、`&[u8]` を渡すと
    // 1 回の memcpy で済む (bytes 二重 alloc は避けられないが、cap 分の
    // memory が上限)。
    let uncascaded = raikiri_html::parse(buf.as_slice(), options).map_err(RenderError::Parse)?;
    let cascade = build_cascaded(&uncascaded);
    Ok(HtmlDocument {
        uncascaded,
        cascade,
    })
}
