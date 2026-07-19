//! Regression tests for `parse_html` / `parse_html_with_limits` input byte
//! cap enforcement (raikiri-spike-d9y.3、SEC-HIGH、Option B stopgap)。
//!
//! `parse_html` は hard-coded 32 MiB input byte cap を持ち、cap 超過は
//! `RenderError::LimitExceeded { kind: LimitKind::AggregateBytes, .. }` として
//! 返る。html5ever は truncated input を silently accept するため、単純
//! `Read::take` だけでは cap 到達を検出できない。実装は
//! `take(cap + 1) + read_to_end` の "+1 probe" pattern (crates/raikiri/src/parse.rs)。
//!
//! Follow-up: `raikiri-spike-4kw` (Option A、`RenderLimits::max_input_bytes` +
//! `LimitKind::InputBytes` へ差し替え、wall/traits crossing 要 human approve)。

use std::io::Read;

use raikiri::{ParseOptions, parse_html, parse_html_with_limits};
use raikiri_traits::{LimitKind, RenderError, RenderLimits};

/// hard-coded cap を test 側でも参照するための local mirror
/// (parse.rs 側の private const、public 化は Option A 移行と一緒に検討)。
const INPUT_BYTES_CAP: u64 = 32 * 1024 * 1024;

fn opts() -> ParseOptions<'static> {
    ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
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
/// 完全に一致することを pin (parse_html は with_limits の thin wrapper なので、
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
/// "+1 probe" が exactly-at-cap を誤って reject しないことを pin
/// (境界条件、buf.len() == INPUT_BYTES_CAP の場合は accept)。
///
/// NB: 32 MiB alloc + parse は memory / time 的に重いが、境界検証は SEC-HIGH
/// regression test の core なので implicitly ignored にしない。
#[test]
fn parse_html_accepts_input_exactly_at_cap() {
    // 32 MiB の valid HTML: <!-- ... --> comment で埋める。html5ever が comment
    // を dropping する pipeline のみ通せば充分 (parse 結果自体は使わない)。
    //
    // "<!--" (4) + payload + "-->" (3) = INPUT_BYTES_CAP bytes に調整。
    let cap_usize = usize::try_from(INPUT_BYTES_CAP).expect("cap fits usize on test targets");
    let payload_len = cap_usize - "<!--".len() - "-->".len();
    let mut buf = Vec::with_capacity(cap_usize);
    buf.extend_from_slice(b"<!--");
    buf.resize(cap_usize - "-->".len(), b'a');
    buf.extend_from_slice(b"-->");
    debug_assert_eq!(buf.len(), cap_usize, "test setup: input len must equal cap");
    let _ = payload_len; // used only for buf sizing math above

    let result = parse_html(&buf[..], &opts());
    assert!(
        result.is_ok(),
        "input == cap must be accepted (boundary), got {:?}",
        result.as_ref().err()
    );
}

/// Input が cap + 1 byte のとき、`LimitExceeded { kind: AggregateBytes, .. }`
/// を返す。SEC-HIGH d9y.3 core assertion。
///
/// `std::io::repeat` で streaming 生成し、over-cap の 32 MiB 超 alloc を
/// test source 側では避ける (parse_html_with_limits 内部の read_to_end は
/// cap + 1 bytes = ≈ 32 MiB alloc、これは避けられない)。
#[test]
fn parse_html_rejects_input_one_byte_over_cap() {
    // 32 MiB + 1 byte を streaming で提供 (test source 側 memory 節約)。
    let source = std::io::repeat(b'a').take(INPUT_BYTES_CAP + 1);
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
                LimitKind::AggregateBytes,
                "Option B stopgap は AggregateBytes を re-use (follow-up 4kw で InputBytes 化)"
            );
            assert_eq!(
                limit, INPUT_BYTES_CAP,
                "limit field は hard-coded INPUT_BYTES_CAP を報告"
            );
            assert!(
                actual > INPUT_BYTES_CAP,
                "actual (= observed) は cap を超えていなければならない、got {actual}"
            );
        }
        other => panic!("expected RenderError::LimitExceeded, got {other:?}"),
    }
}

/// `parse_html_with_limits` 経路でも同じく over-cap で LimitExceeded を返す。
///
/// `RenderLimits` の中身は Option B stopgap では consult されず、hard-coded
/// cap のみが effective である契約を pin。
#[test]
fn parse_html_with_limits_rejects_input_over_cap_regardless_of_limits_field() {
    // Consumer が「大き目」の limits を builder で作っても、Option B stopgap
    // では hard-coded cap しか見ないので reject される。
    let limits = RenderLimits::builder()
        .max_aggregate_bytes(Some(u64::MAX))
        .build();

    let source = std::io::repeat(b'b').take(INPUT_BYTES_CAP + 1);
    let err = parse_html_with_limits(source, &opts(), limits)
        .expect_err("over-cap input must be rejected regardless of limits field values");

    assert!(
        matches!(
            err,
            RenderError::LimitExceeded {
                kind: LimitKind::AggregateBytes,
                ..
            }
        ),
        "expected LimitExceeded/AggregateBytes, got {err:?}"
    );
}
