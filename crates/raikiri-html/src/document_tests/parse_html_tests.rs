use super::*;

#[test]
fn parse_html_returns_html_document_with_cascade_populated() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let doc = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html should succeed");
    assert!(
        !doc.cascade().computed.is_empty(),
        "parse_html output must have populated cascade"
    );
    assert_eq!(
        doc.cascade().computed.len(),
        doc.dom().node_count(),
        "cascade / dom node_count invariant"
    );
}

#[test]
fn parse_html_propagates_parse_error_from_io() {
    struct FailingReader;
    impl std::io::Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("boom"))
        }
    }
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let err = parse_html(FailingReader, &opts).expect_err("must fail on reader error");
    assert!(
        matches!(err, RenderError::Parse(ParseError::Io(_))),
        "expected RenderError::Parse(ParseError::Io), got {err:?}"
    );
}

#[test]
fn parse_html_propagates_utf8_error() {
    // 0x80 alone is a UTF-8 continuation byte and invalid UTF-8.
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let err = parse_html(&[0x80u8, 0x80, 0x80][..], &opts).expect_err("must fail on invalid UTF-8");
    assert!(
        matches!(err, RenderError::Parse(ParseError::Encoding { .. })),
        "expected RenderError::Parse(ParseError::Encoding), got {err:?}"
    );
}

#[test]
fn parse_html_baked_cascade_matches_manual_build_cascaded() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    // Check that both cascade paths produce the same result (contract:
    // parse_html calls build_cascaded internally).
    let via_parse_html = parse_html(&b"<p>Hi</p>"[..], &opts).expect("parse_html");
    let via_manual = {
        let uncascaded = crate::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
        build_cascaded(&uncascaded)
    };
    assert_eq!(
        via_parse_html.cascade().computed.len(),
        via_manual.computed.len(),
        "parse_html と手動 build_cascaded で cascade node 数が一致"
    );
}

// `HtmlDocument.uncascaded` is `pub(crate)` (not public to consumers).
// Verifying that `RenderLimits::max_parse_warnings` is actually consulted by
// `RaikiriTreeSink` during parsing therefore requires a white-box test
// inside this crate (to access `doc.uncascaded.warnings`).

#[test]
fn parse_html_with_limits_consults_custom_max_parse_warnings() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    // `</x>` has no matching start element, so html5ever's error recovery
    // algorithm reports roughly one nonfatal parse error per closing tag.
    // This matches the input shape used in similar raikiri_html tests. With cap=5,
    // last_real_slot=4, allowing four real warnings plus one synthetic warning:
    // five in total (the reserved-last-slot contract; see the docs for
    // `RaikiriTreeSink::parse_error`).
    let malformed = b"</x>".repeat(50);

    let strict_limits = RenderLimits::builder().max_parse_warnings(Some(5)).build();
    let strict_doc = parse_html_with_limits(malformed.as_slice(), &opts, strict_limits)
        .expect("malformed input still recovers");
    assert_eq!(
        strict_doc.uncascaded.warnings.len(),
        5,
        "custom max_parse_warnings=5 must be consulted and yield exactly 4 real + 1 synthetic warning"
    );
    match &strict_doc.uncascaded.warnings.last().expect("cap > 0").kind {
        raikiri_traits::WarningKind::HtmlParseError { message } => {
            assert!(
                message.contains("5-warning cap"),
                "last entry must be the synthetic suppression notice naming the runtime cap (5), got: {message:?}"
            );
        }
        other => panic!("expected HtmlParseError, got {other:?}"),
    }

    // Contrast: parsing the same input with the default cap (1024) records
    // more than 5 warnings — this checks that the cap value is actually read
    // from RenderLimits (not just coincidentally a fixed small value).
    let default_doc = parse_html_with_limits(malformed.as_slice(), &opts, RenderLimits::default())
        .expect("malformed input still recovers");
    assert!(
        default_doc.uncascaded.warnings.len() > strict_doc.uncascaded.warnings.len(),
        "default cap must allow strictly more warnings than the custom cap=5, got default={} strict={}",
        default_doc.uncascaded.warnings.len(),
        strict_doc.uncascaded.warnings.len()
    );
}

#[test]
fn parse_html_with_limits_zero_max_parse_warnings_records_one_synthetic_entry() {
    // cap=0 has no slot for a real warning, but must still record exactly
    // one synthetic suppression entry when parse errors actually occur,
    // consistent with cap>0's trip-and-record-suppression semantics.
    // Pins that this boundary is reachable via `RenderLimits`, not just
    // via `RaikiriTreeSink::new()` directly.
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let malformed = b"</x>".repeat(50);

    let limits = RenderLimits::builder().max_parse_warnings(Some(0)).build();
    let doc = parse_html_with_limits(malformed.as_slice(), &opts, limits)
        .expect("malformed input still recovers");
    assert_eq!(
        doc.uncascaded.warnings.len(),
        1,
        "max_parse_warnings=Some(0) must record exactly one synthetic suppression entry, got {}",
        doc.uncascaded.warnings.len()
    );
}

#[test]
fn parse_html_with_limits_none_max_parse_warnings_disables_cap() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    // 2,000 `</x>` closing tags reliably reach the default cap (1024),
    // as confirmed by similar tests in raikiri_html.
    let malformed = b"</x>".repeat(2_000);

    let mut limits = RenderLimits::default();
    limits.max_parse_warnings = None;
    let doc = parse_html_with_limits(malformed.as_slice(), &opts, limits)
        .expect("malformed input still recovers");

    assert!(
        doc.uncascaded.warnings.len() > 1024,
        "max_parse_warnings=None must not cap warnings at the default 1024, got {}",
        doc.uncascaded.warnings.len()
    );
}
