//! Regression tests for `parse_html` / `parse_html_with_limits` input byte
//! cap enforcement (promotion of an earlier hard-coded stopgap to a
//! configurable field)。
//!
//! `parse_html` は [`RenderLimits::default().max_input_bytes`] = 32 MiB
//! input byte cap を持ち、cap 超過は
//! `RenderError::LimitExceeded { kind: LimitKind::InputBytes, .. }` として
//! 返る。html5ever は truncated input を silently accept するため、単純
//! `Read::take` だけでは cap 到達を検出できない。実装は
//! `take(cap + 1) + read_to_end` の "+1 probe" pattern (crates/raikiri-html/src/document_parse.rs)。
//!
//! Consumer は [`RenderLimitsBuilder::max_input_bytes`] (または field への
//! 直接代入) で cap を調整、`None` で無効化できる (`None` の security 上の
//! 含意は `RenderLimits::max_input_bytes` field doc を参照)。

use std::io::Read;

use raikiri::{ParseOptions, parse_html, parse_html_with_limits};
use raikiri_traits::{LimitKind, RenderError, RenderLimits};

/// Default cap を test 側でも参照するための local mirror
/// (`RenderLimits::default().max_input_bytes` に合わせる、SEC-HIGH regression
/// が破れると default が変わった signal)。
const DEFAULT_INPUT_BYTES_CAP: u64 = 32 * 1024 * 1024;

fn opts() -> ParseOptions<'static> {
    ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
}

/// `RenderLimits::default().max_input_bytes` は SEC-HIGH の旧 stopgap
/// と一致する 32 MiB を継承していることを check (promotion
/// が behavior 不変であることの regression guard)。
#[test]
fn render_limits_default_input_cap_matches_d9y3_stopgap() {
    assert_eq!(
        RenderLimits::default().max_input_bytes,
        Some(DEFAULT_INPUT_BYTES_CAP),
        "default は旧 stopgap の hard-coded 32 MiB を継承"
    );
}

/// Small (< cap) input: 正常 parse。cascade も populate される。
///
/// Cap enforcement が small input を誤って reject しないことを pin。
#[test]
fn parse_html_accepts_input_well_below_cap() {
    let html = b"<html><body><p>Hi</p></body></html>";
    let doc = parse_html(&html[..], &opts()).expect("small input must parse");
    assert!(
        !doc.cascade().computed.is_empty(),
        "cascade must be populated for small under-cap input"
    );
}

/// `parse_html_with_limits` も同じく small input を parse する。
///
/// `RenderLimits::default()` を渡した場合の behavior が `parse_html` と
/// 完全に一致することを check (parse_html は with_limits の thin wrapper なので、
/// この test が失敗すると wrapper 契約が壊れている)。
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

/// Input が **正確に** cap = 32 MiB のとき、cap 内 accept される。
///
/// "+1 probe" が exactly-at-cap を誤って reject しないことを check
/// (境界条件、buf.len() == cap の場合は accept)。
///
/// NB: 32 MiB alloc + parse は memory / time 的に重いが、境界検証は SEC-HIGH
/// regression test の core なので implicitly ignored にしない。
#[test]
fn parse_html_accepts_input_exactly_at_cap() {
    // 32 MiB の valid HTML: <!-- ... --> comment で埋める。html5ever が comment
    // を dropping する pipeline のみ通せば充分 (parse 結果自体は使わない)。
    //
    // "<!--" (4) + payload + "-->" (3) = DEFAULT_INPUT_BYTES_CAP bytes に調整。
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

/// Input が cap + 1 byte のとき、`LimitExceeded { kind: InputBytes, .. }`
/// を返す。SEC-HIGH core assertion (kind が InputBytes に昇格済み)。
///
/// `std::io::repeat` で streaming 生成し、over-cap の 32 MiB 超 alloc を
/// test source 側では避ける (parse_html_with_limits 内部の read_to_end は
/// cap + 1 bytes = ≈ 32 MiB alloc、これは避けられない)。
#[test]
fn parse_html_rejects_input_one_byte_over_cap() {
    // 32 MiB + 1 byte を streaming で提供 (test source 側 memory 節約)。
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

/// Custom cap の enforcement check: `Some(N)` を渡すと N byte で reject される
/// (`limits.max_input_bytes` が実際に consult されている
/// ことを cheap な small input で証明)。
///
/// Cap = 100, input = 101 byte → reject。
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

/// Custom cap の accept 側 check: cap = 200, input = 101 byte → accept
/// (contrast: 同じ 101 byte input が cap=100 では reject、cap=200 では accept、
/// これで cap field が actually consulted であることを確定させる)。
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

/// `max_input_bytes = None` は cap を無効化する (Consumer が明示的に opt-out
/// した場合の unbounded read path)。
///
/// Contrast test: 同じ 101 byte input が cap=100 では reject、cap=None では
/// accept。default (32 MiB) では 101 byte は無関係に accept されるので、
/// この test が check するのは "None field が実際に unbounded 経路を選ぶ"
/// ことである (cap=100 rejection と対比してのみ意味を持つ)。
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

    // Contrast: 同じ input を cap=100 で試すと reject される (None 経路の意味を pin)。
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

/// `RenderLimitsBuilder::max_input_bytes` の compile + runtime pin。
///
/// Consumer が builder pattern で cap を tune できる契約 (`Some(cap)` / `None`
/// 両経路の設定が builder + field 直接代入と equivalent であること)。
#[test]
fn render_limits_builder_max_input_bytes_roundtrip() {
    let via_builder = RenderLimits::builder()
        .max_input_bytes(Some(64 * 1024 * 1024))
        .build();
    assert_eq!(via_builder.max_input_bytes, Some(64 * 1024 * 1024));

    // Direct field write pattern (`with_*` ergonomic を持たない sibling
    // convention に揃えている)。
    let mut via_field = RenderLimits::default();
    via_field.max_input_bytes = Some(64 * 1024 * 1024);
    assert_eq!(via_field.max_input_bytes, Some(64 * 1024 * 1024));

    // None も builder 経由で設定可能。
    let unbounded = RenderLimits::builder().max_input_bytes(None).build();
    assert_eq!(unbounded.max_input_bytes, None);
}
