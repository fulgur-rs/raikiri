//! Regression tests for enforcing the `parse_html` / `parse_html_with_limits`
//! input-byte cap (a formerly hard-coded stopgap, now configurable).
//!
//! `parse_html` has a 32 MiB cap, [`RenderLimits::default().max_input_bytes`].
//! Inputs beyond this cap return
//! `RenderError::LimitExceeded { kind: LimitKind::InputBytes, .. }`.
//! Because html5ever silently accepts truncated input, `Read::take` alone
//! cannot detect a cap violation. The implementation uses the "+1 probe":
//! `take(cap + 1) + read_to_end` (crates/raikiri-html/src/document_parse.rs).
//!
//! Consumers can adjust the cap with [`RenderLimitsBuilder::max_input_bytes`]
//! (or direct field assignment) and disable it with `None`. See the
//! `RenderLimits::max_input_bytes` field docs for the security implications.

use std::io::Read;

use raikiri::{ParseOptions, parse_html, parse_html_with_limits};
use raikiri_traits::{LimitKind, RenderError, RenderLimits};

/// Local copy of the default cap for tests. If it diverges from
/// `RenderLimits::default().max_input_bytes`, the SEC-HIGH regression fails,
/// signaling that the default has changed.
const DEFAULT_INPUT_BYTES_CAP: u64 = 32 * 1024 * 1024;

fn opts() -> ParseOptions<'static> {
    ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
}

/// Check that `RenderLimits::default().max_input_bytes` retains the
/// original SEC-HIGH stopgap of 32 MiB after becoming configurable.
#[test]
fn render_limits_default_input_cap_matches_d9y3_stopgap() {
    assert_eq!(
        RenderLimits::default().max_input_bytes,
        Some(DEFAULT_INPUT_BYTES_CAP),
        "default は旧 stopgap の hard-coded 32 MiB を継承"
    );
}

/// An input smaller than the cap parses and populates the cascade.
///
/// Guard against rejecting valid small inputs.
#[test]
fn parse_html_accepts_input_well_below_cap() {
    let html = b"<html><body><p>Hi</p></body></html>";
    let doc = parse_html(&html[..], &opts()).expect("small input must parse");
    assert!(
        !doc.cascade().computed.is_empty(),
        "cascade must be populated for small under-cap input"
    );
}

/// `parse_html_with_limits` also accepts a small input.
///
/// With `RenderLimits::default()`, it must behave exactly like `parse_html`,
/// which is a thin wrapper around the limits-aware function.
#[test]
fn parse_html_with_limits_default_matches_parse_html_for_small_input() {
    let html = b"<html><body><p>Hi</p></body></html>";
    let via_wrapper = parse_html(&html[..], &opts()).expect("parse_html small input");
    let via_direct = parse_html_with_limits(&html[..], &opts(), RenderLimits::default())
        .expect("parse_html_with_limits small input");
    assert_eq!(
        via_wrapper.cascade().computed.len(),
        via_direct.cascade().computed.len(),
        "parse_html は parse_html_with_limits の thin wrapper 契約"
    );
}

/// An input of **exactly** 32 MiB remains within the cap.
///
/// The "+1 probe" must accept this boundary (`buf.len() == cap`).
///
/// Allocating and parsing 32 MiB is costly, but this boundary is central
/// to the SEC-HIGH regression and must not be silently skipped.
#[test]
fn parse_html_accepts_input_exactly_at_cap() {
    // Fill a valid 32 MiB HTML document with a <!-- ... --> comment.
    // html5ever only needs to process and discard the comment; the parsed
    // result itself is not used.
    // "<!--" (4) + payload + "-->" (3) = DEFAULT_INPUT_BYTES_CAP bytes.
    let cap_usize =
        usize::try_from(DEFAULT_INPUT_BYTES_CAP).expect("cap fits usize on test targets");
    let mut buf = Vec::with_capacity(cap_usize);
    buf.extend_from_slice(b"<!--");
    buf.resize(cap_usize - "-->".len(), b'a');
    buf.extend_from_slice(b"-->");
    debug_assert_eq!(buf.len(), cap_usize, "test setup: input len must equal cap");

    let result = parse_html(&buf[..], &opts());
    assert!(
        result.is_ok(),
        "input == cap must be accepted (boundary), got {:?}",
        result.as_ref().err()
    );
}

/// At cap + 1 bytes, return `LimitExceeded { kind: InputBytes, .. }`.
/// This is the core SEC-HIGH assertion after introducing the InputBytes kind.
///
/// Generate input with `std::io::repeat` to avoid allocating over 32 MiB
/// in the test source. The internal `read_to_end` still necessarily
/// allocates approximately 32 MiB (cap + 1 bytes).
#[test]
fn parse_html_rejects_input_one_byte_over_cap() {
    // Stream 32 MiB + 1 byte to save memory in the test source.
    let source = std::io::repeat(b'a').take(DEFAULT_INPUT_BYTES_CAP + 1);
    let err =
        parse_html(source, &opts()).expect_err("input > cap must be rejected as LimitExceeded");

    match err {
        RenderError::LimitExceeded {
            kind,
            limit,
            actual,
        } => {
            assert_eq!(
                kind,
                LimitKind::InputBytes,
                "kind は InputBytes (旧 stopgap の AggregateBytes からの migration)"
            );
            assert_eq!(
                limit, DEFAULT_INPUT_BYTES_CAP,
                "limit field は default max_input_bytes を報告"
            );
            assert!(
                actual > DEFAULT_INPUT_BYTES_CAP,
                "actual (= observed) は cap を超えていなければならない、got {actual}"
            );
        }
        other => panic!("expected RenderError::LimitExceeded, got {other:?}"),
    }
}

/// A custom `Some(N)` cap rejects input above N bytes, proving with
/// cheap, small input that `limits.max_input_bytes` is actually used.
///
/// Cap = 100, input = 101 bytes: reject.
#[test]
fn parse_html_with_limits_custom_cap_rejects_over_cap() {
    let limits = RenderLimits::builder().max_input_bytes(Some(100)).build();
    let input = vec![b'a'; 101];

    let err = parse_html_with_limits(input.as_slice(), &opts(), limits)
        .expect_err("input > custom cap must be rejected");

    match err {
        RenderError::LimitExceeded {
            kind,
            limit,
            actual,
        } => {
            assert_eq!(kind, LimitKind::InputBytes, "kind = InputBytes");
            assert_eq!(limit, 100, "limit field は custom cap を報告");
            assert!(
                actual > 100,
                "actual は custom cap を超えていなければならない、got {actual}"
            );
        }
        other => panic!("expected RenderError::LimitExceeded, got {other:?}"),
    }
}

/// A custom cap also accepts input below the limit: with cap = 200,
/// 101 bytes are accepted, whereas the same input is rejected at cap = 100.
/// This contrast confirms that the cap field is actually consulted.
#[test]
fn parse_html_with_limits_custom_cap_accepts_under_cap() {
    let limits = RenderLimits::builder().max_input_bytes(Some(200)).build();
    let mut input = Vec::from(&b"<!--"[..]);
    input.resize(101 - "-->".len(), b'a');
    input.extend_from_slice(b"-->");
    debug_assert_eq!(input.len(), 101);

    let result = parse_html_with_limits(input.as_slice(), &opts(), limits);
    assert!(
        result.is_ok(),
        "input < custom cap must be accepted, got {:?}",
        result.as_ref().err()
    );
}

/// `max_input_bytes = None` disables the cap, allowing the consumer
/// to explicitly opt into unbounded reads.
///
/// The same 101-byte input is rejected at cap = 100 but accepted at
/// cap = None. The default 32 MiB cap also accepts 101 bytes, so only
/// the contrast with cap = 100 tests the unbounded path.
#[test]
fn parse_html_with_limits_none_disables_cap() {
    let mut limits = RenderLimits::default();
    limits.max_input_bytes = None;
    let mut input = Vec::from(&b"<!--"[..]);
    input.resize(101 - "-->".len(), b'a');
    input.extend_from_slice(b"-->");
    debug_assert_eq!(input.len(), 101);

    let result = parse_html_with_limits(input.as_slice(), &opts(), limits);
    assert!(
        result.is_ok(),
        "input must be accepted with max_input_bytes=None, got {:?}",
        result.as_ref().err()
    );

    // The same input is rejected at cap = 100, establishing the contrast with None.
    let strict = RenderLimits::builder().max_input_bytes(Some(100)).build();
    let strict_err =
        parse_html_with_limits(input.as_slice(), &opts(), strict).expect_err("strict cap rejects");
    assert!(matches!(
        strict_err,
        RenderError::LimitExceeded {
            kind: LimitKind::InputBytes,
            ..
        }
    ));
}

/// Compile-time and runtime check of `RenderLimitsBuilder::max_input_bytes`.
///
/// Consumers can tune the cap through the builder. Both `Some(cap)` and
/// `None` must be equivalent to direct field assignment.
#[test]
fn render_limits_builder_max_input_bytes_roundtrip() {
    let via_builder = RenderLimits::builder()
        .max_input_bytes(Some(64 * 1024 * 1024))
        .build();
    assert_eq!(via_builder.max_input_bytes, Some(64 * 1024 * 1024));

    // Direct field assignment follows the convention of sibling types
    // without a `with_*` convenience method.
    let mut via_field = RenderLimits::default();
    via_field.max_input_bytes = Some(64 * 1024 * 1024);
    assert_eq!(via_field.max_input_bytes, Some(64 * 1024 * 1024));

    // The builder also accepts None.
    let unbounded = RenderLimits::builder().max_input_bytes(None).build();
    assert_eq!(unbounded.max_input_bytes, None);
}
