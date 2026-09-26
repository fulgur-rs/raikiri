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
    // 0x80 は UTF-8 continuation byte 単独、invalid UTF-8
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
    // 2 経路の cascade が同じ結果を出すことを check (parse_html は
    // build_cascaded を内部で呼んでいる契約)
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

// `HtmlDocument.uncascaded` は `pub(crate)` (Consumer 向けには非公開) の
// ため、`RenderLimits::max_parse_warnings` が実際に parse 中の
// `RaikiriTreeSink` に consult されていることの検証は、この crate 内部の
// white-box test としてのみ書ける (`doc.uncascaded.warnings` へのアクセス
// が必要)。

#[test]
fn parse_html_with_limits_consults_custom_max_parse_warnings() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    // `</x>` は closing tag に対応する開始要素が無いため、html5ever の
    // エラー回復アルゴリズムが 1 個あたり概ね 1 個の非致命 parse error を
    // 報告する (raikiri_html crate の同種 test と同じ input shape)。cap=5
    // → last_real_slot=4 なので real warning 4 件 + synthetic 1 件の
    // 計 5 件で頭打ちになる (reserved-last-slot 契約、
    // `RaikiriTreeSink::parse_error` doc 参照)。
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

    // Contrast: 同じ input を default cap (1024) で parse すると 5 件より
    // 多く記録される — cap 値が実際に RenderLimits から読まれていること
    // (固定の小さい値に偶然収まっただけではないこと) を check する。
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
    // 2,000 個の `</x>` は default cap (1024) 下では確実に cap に到達する
    // 入力サイズ (raikiri_html crate の同種 test で確認済み)。
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
