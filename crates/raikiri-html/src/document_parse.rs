//! `parse_html`: orchestrator that builds a cascaded [`HtmlDocument`] from an HTML
//! byte stream.
//!
//! Equivalent to the public API in spec §L1060. Internal pipeline:
//! [`crate::parse`](fn@crate::parse) → rule-tree build and first-page cascade → assemble.
//! The cascade currently always returns `Ok`,
//! so parse and limit errors can propagate.
//!
//! # Input byte cap
//!
//! `parse_html_with_limits` reads [`RenderLimits::max_input_bytes`] and bounds
//! input reads. This closes SEC-HIGH: an unbounded-read DoS in `parse_html`
//! where attacker-supplied HTML of arbitrary size could exhaust memory.
//! The implementation probes with `Read::take(cap + 1)` and `read_to_end`:
//! after reading, it checks whether `buf.len() > cap` and returns
//! `RenderError::LimitExceeded { kind: LimitKind::InputBytes, .. }` if so.
//! A plain `take(cap)` would not detect the limit because html5ever silently
//! accepts truncated input.
//!
//! Initially, a hard-coded `INPUT_BYTES_CAP` and reused
//! `LimitKind::AggregateBytes` were a stopgap for SEC-HIGH. These were later
//! replaced by `RenderLimits::max_input_bytes: Option<u64>` (default
//! `Some(32 * 1024 * 1024)`) and `LimitKind::InputBytes` in the traits API.
//! The default is the same as the old stopgap, so consumers passing
//! `RenderLimits::default()` see no behavior change. To change the cap, use
//! [`RenderLimitsBuilder::max_input_bytes`](raikiri_traits::RenderLimitsBuilder::max_input_bytes)
//! (or assign the field directly). To disable it, set `max_input_bytes = None`.
//! **Disabling the cap reopens the SEC-HIGH DoS**; see the field's Security
//! note.

use std::io::Read;

use crate::{ParseOptions, RaikiriTreeSink, effective_document_base_url, parse_with_sink};
use raikiri_traits::{LimitKind, ParseError, RenderError, RenderLimits};

use raikiri_style::PageContextQuery;

use crate::HtmlDocument;
use crate::cascade::build_rule_tree;

/// Parse an HTML byte stream and return a fully cascaded [`HtmlDocument`].
///
/// A thin wrapper around [`parse_html_with_limits`] that passes
/// [`RenderLimits::default()`]. The existing `parse_html` API therefore
/// enforces the default 32 MiB input cap
/// (`RenderLimits::default().max_input_bytes = Some(32 * 1024 * 1024)`)
/// and the default 1,000,000-node DOM cap.
///
/// # Errors
///
/// - `RenderError::Parse(ParseError::Io)`: reading `input` failed.
/// - `RenderError::Parse(ParseError::Encoding)`: input is not valid UTF-8.
/// - `RenderError::Parse(ParseError::*)`: html5ever returned a parse error.
/// - `RenderError::LimitExceeded { kind: LimitKind::InputBytes, .. }`:
///   input byte count exceeds [`RenderLimits::max_input_bytes`].
/// - `RenderError::LimitExceeded { kind: LimitKind::DomNodes, .. }`:
///   parsed DOM node count exceeds [`RenderLimits::max_dom_nodes`].
///
/// The cascade is currently infallible (unwrapped with `.expect`).
///
/// # Example
///
/// ```
/// use raikiri_html::{parse_html, ParseOptions};
///
/// let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
/// let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse");
/// assert!(!doc.cascade().computed.is_empty());
/// ```
pub fn parse_html<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
) -> Result<HtmlDocument, RenderError> {
    parse_html_with_limits(input, options, RenderLimits::default())
}

/// Variant of `parse_html` that explicitly accepts `RenderLimits`.
/// It consults [`RenderLimits::max_input_bytes`]:
///
/// * With `Some(cap)`, probe up to `cap + 1` input bytes and return
///   [`RenderError::LimitExceeded`] early if the input exceeds `cap`.
/// * With `None`, read input without a limit. Only explicit consumer opt-out
///   enables this; the fail-closed default remains 32 MiB.
///
/// [`RenderLimits::max_parse_warnings`] is passed directly to `RaikiriTreeSink`.
/// It caps nonfatal html5ever parse errors recorded as warnings. Unlike the input
/// byte cap, this does not stop early. Excess errors are dropped, and one
/// synthetic warning is added instead.
///
/// [`RenderLimits::max_dom_nodes`] is checked after parsing and before rule-tree
/// construction or cascading. `None` disables this check.
/// Other `limits.*` fields (`max_aggregate_bytes` / etc.) are **not consulted**
/// by `parse_html_with_limits` yet; they are reserved for downstream layers.
///
/// # Implementation
///
/// With `Some(cap)`, `input.by_ref().take(cap + 1).read_to_end(&mut buf)`
/// performs a bounded probe. If `buf.len() > cap`, input exceeded the cap.
/// A plain `Read::take(cap)` truncates silently at the cap, and html5ever
/// parses truncated input without warning; this "+1 probe" is needed to
/// detect an over-limit input.
///
/// With `None`, `input.read_to_end(&mut buf)` reads without a limit.
///
/// # Errors
///
/// - `RenderError::Parse(ParseError::Io)`: reading `input` failed.
/// - `RenderError::Parse(ParseError::Encoding)`: input is not valid UTF-8.
/// - `RenderError::Parse(ParseError::*)`: html5ever returned a parse error.
/// - `RenderError::LimitExceeded { kind: LimitKind::InputBytes, limit, actual }`:
///   input bytes exceeded `limits.max_input_bytes.unwrap()`. `limit` is
///   the configured cap; `actual` only establishes that the cap was exceeded.
///   The true input size is unknown because detection stops just past the cap.
/// - `RenderError::LimitExceeded { kind: LimitKind::DomNodes, limit, actual }`:
///   the parsed DOM node count exceeded `limits.max_dom_nodes.unwrap()`.
///   `actual` is the full arena node count, including the virtual root.
pub fn parse_html_with_limits<R: Read>(
    mut input: R,
    options: &ParseOptions<'_>,
    limits: RenderLimits,
) -> Result<HtmlDocument, RenderError> {
    let mut buf: Vec<u8> = Vec::new();
    match limits.max_input_bytes {
        Some(cap) => {
            // "+1 probe": reading cap + 1 bytes proves that input exceeds cap.
            // Use saturating_add to avoid overflow when cap == u64::MAX.
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
            // The consumer explicitly disabled the cap (the fail-closed default is
            // 32 MiB; this branch is reached only through opt-out).
            input
                .read_to_end(&mut buf)
                .map_err(|e| RenderError::Parse(ParseError::Io(e)))?;
        }
    }

    // Below cap: pass the materialized slice to crate::parse_with_sink.
    // The crate::parse thin-wrapper path always uses
    // `RaikiriTreeSink::default()`, so it cannot consult
    // `limits.max_parse_warnings`; construct the sink explicitly here.
    // parse_with_sink internally calls `read_to_end`. Passing `&[u8]` makes
    // one memcpy; a second allocation is unavoidable, but the cap bounds memory use.
    let sink = RaikiriTreeSink::new(limits.max_parse_warnings);
    let uncascaded = parse_with_sink(buf.as_slice(), sink, options).map_err(RenderError::Parse)?;
    if let Some(limit) = limits.max_dom_nodes {
        let actual = uncascaded.dom.node_count() as u64;
        if actual > limit {
            return Err(RenderError::LimitExceeded {
                kind: LimitKind::DomNodes,
                limit,
                actual,
            });
        }
    }
    let effective_base_url = effective_document_base_url(&uncascaded, options.base_url.as_ref());
    let mut first_page = PageContextQuery::default();
    first_page.is_first = true;
    first_page.is_right = true;
    let rule_tree = build_rule_tree(&uncascaded);
    let media_context = raikiri_style::MediaContext::default();
    let font_faces = rule_tree.font_faces_for(&media_context);
    let cascade = raikiri_style::cascade_with_media_context_for_page(
        &uncascaded.dom,
        &rule_tree,
        &media_context,
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
