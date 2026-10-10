//! `parse_html`: orchestrator that builds a cascaded [`HtmlDocument`] from an HTML
//! byte stream.
//!
//! Equivalent to the public API in spec §L1060. Internal pipeline:
//! [`crate::parse`](fn@crate::parse) → rule-tree build and first-page cascade → assemble.
//! Parse, limit and cascade errors propagate.
//!
//! # Input byte cap
//!
//! `parse_html_with_limits` reads [`RenderLimits::max_input_bytes`] and bounds
//! input reads. This closes SEC-HIGH: an unbounded-read DoS in `parse_html`
//! where attacker-supplied HTML of arbitrary size could exhaust memory.
//! Input is parsed as it is read, in chunks, and the reader is wrapped in
//! `Read::take(cap + 1)` with a byte counter. As soon as more than `cap`
//! bytes have been read, parsing stops and the call returns
//! `RenderError::LimitExceeded { kind: LimitKind::InputBytes, .. }`.
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

use crate::{
    ParseOptions, RaikiriTreeSink, UncascadedDocument, effective_document_base_url, parse_with_sink,
};
use raikiri_traits::{LimitKind, RenderError, RenderLimits};

use raikiri_style::{CascadeOptions, PageContextQuery};

use crate::HtmlDocument;
use crate::cascade::build_rule_tree;

/// Parse an HTML byte stream and return a fully cascaded [`HtmlDocument`].
///
/// A thin wrapper around [`parse_html_with_limits`] that passes
/// [`RenderLimits::default()`]. The existing `parse_html` API therefore
/// enforces the default 32 MiB input cap
/// (`RenderLimits::default().max_input_bytes = Some(32 * 1024 * 1024)`),
/// the default 1,000,000-node DOM cap and the default cascade limits.
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
/// - `RenderError::LimitExceeded { kind: LimitKind::Cascade*, .. }`: the
///   cascade passed one of [`RenderLimits::cascade_limits`].
/// - `RenderError::Cascade(_)`: the allocator refused the cascade's result,
///   or the stylesheets have more rules, selectors or declarations than the
///   cascade can number.
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
/// The first-page cascade runs within [`RenderLimits::cascade_limits`], the
/// `max_cascade_*` fields. Other `limits.*` fields (`max_aggregate_bytes` /
/// etc.) are **not consulted** by `parse_html_with_limits`; they are for the
/// layout stages or reserved.
///
/// # Implementation
///
/// Input is not buffered as a whole: it is read in chunks and each chunk is
/// passed to html5ever as soon as it arrives. With `Some(cap)`, the reader is
/// wrapped in `take(cap + 1)` and every byte read is counted; reading byte
/// `cap + 1` stops the parse. A plain `Read::take(cap)` truncates silently at
/// the cap, and html5ever parses truncated input without warning; this
/// "+1 probe" is needed to detect an over-limit input.
///
/// With `None`, the input is read to its end without a limit.
///
/// Because parsing and reading are interleaved, an input that is both
/// invalid UTF-8 and over the cap reports whichever problem comes first.
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
/// - `RenderError::LimitExceeded { kind: LimitKind::Cascade*, .. }`: the
///   cascade passed one of [`RenderLimits::cascade_limits`]; `actual` is the
///   count it stopped at.
/// - `RenderError::Cascade(_)`: the allocator refused the cascade's result,
///   or the stylesheets have more rules, selectors or declarations than the
///   cascade can number.
pub fn parse_html_with_limits<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
    limits: RenderLimits,
) -> Result<HtmlDocument, RenderError> {
    // Construct the sink explicitly: the crate::parse thin-wrapper path
    // always uses `RaikiriTreeSink::default()`, so it cannot consult
    // `limits.max_parse_warnings`.
    let sink = RaikiriTreeSink::new(limits.max_parse_warnings);
    let uncascaded = match limits.max_input_bytes {
        Some(cap) => {
            let mut capped = CappedInput::new(input, cap);
            let parsed = parse_with_sink(&mut capped, sink, options);
            if let Some(actual) = capped.exceeded() {
                return Err(RenderError::LimitExceeded {
                    kind: LimitKind::InputBytes,
                    limit: cap,
                    actual,
                });
            }
            parsed
        }
        // The consumer explicitly disabled the cap (the fail-closed default
        // is 32 MiB; this branch is reached only through opt-out).
        None => parse_with_sink(input, sink, options),
    }
    .map_err(RenderError::Parse)?;
    assemble_document(uncascaded, options, &limits)
}

/// Check the DOM node cap on a parsed document and run its first-page
/// cascade.
pub(crate) fn assemble_document(
    uncascaded: UncascadedDocument,
    options: &ParseOptions<'_>,
    limits: &RenderLimits,
) -> Result<HtmlDocument, RenderError> {
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
    let mut cascade_options = CascadeOptions::default();
    cascade_options.limits = limits.cascade_limits();
    let cascade = raikiri_style::cascade_with_options(
        &uncascaded.dom,
        &rule_tree,
        &media_context,
        &first_page,
        &cascade_options,
    )?;
    Ok(HtmlDocument {
        uncascaded,
        cascade,
        font_faces,
        effective_base_url,
    })
}

/// Reader that stops with an error once more than `cap` bytes were read.
///
/// The inner reader is limited to `cap + 1` bytes, so the reported count is
/// exactly `cap + 1` when the cap is exceeded, and nothing past that byte is
/// ever requested from the caller's reader.
struct CappedInput<R> {
    inner: std::io::Take<R>,
    cap: u64,
    read: u64,
}

impl<R: Read> CappedInput<R> {
    fn new(input: R, cap: u64) -> Self {
        // Use saturating_add to avoid overflow when cap == u64::MAX.
        Self {
            inner: input.take(cap.saturating_add(1)),
            cap,
            read: 0,
        }
    }

    /// The number of bytes read, if it exceeded the cap.
    fn exceeded(&self) -> Option<u64> {
        (self.read > self.cap).then_some(self.read)
    }
}

impl<R: Read> Read for CappedInput<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.read += read as u64;
        if self.read > self.cap {
            return Err(std::io::Error::other("input byte cap exceeded"));
        }
        Ok(read)
    }
}
