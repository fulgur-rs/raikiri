use super::*;

#[test]
fn empty_content_yields_empty_sets() {
    let tracked = TrackedWpt::parse("# only a comment\n", "tracked-wpt.txt").unwrap();
    assert!(tracked.is_empty());

    let ki = KnownIssues::parse("", "known-issues.txt").unwrap();
    assert!(ki.is_empty());

    let base = Baseline::parse("# header\n\n", "raikiri-baseline.txt").unwrap();
    assert!(base.is_empty());

    let dep = Deprecated::parse("", "deprecated.txt").unwrap();
    assert!(dep.is_empty());

    let (q, q_errs) = Quarantine::parse("# 8-col format follows\n", "quarantine.txt");
    assert!(q_errs.is_empty());
    assert!(q.is_empty());
    let (expected, expected_errs) = ExpectedFailures::parse("# header\n", "expected-failures.txt");
    assert!(expected_errs.is_empty());
    assert!(expected.is_empty());
}

#[test]
fn expected_failure_parses_exact_query_ids_and_lookup_does_not_normalize() {
    let (expected, errors) = ExpectedFailures::parse(
        "css/css-text/spacing.html?lang=ja | Japanese spacing differs | raikiri-spike-6qrnr.15.15 | 2026-10-04 | 2026-11-04\n",
        "expected-failures.txt",
    );

    assert!(errors.is_empty(), "unexpected parser errors: {errors:?}");
    assert_eq!(expected.entries.len(), 1);
    let entry = expected
        .get("css/css-text/spacing.html?lang=ja")
        .expect("exact variant is present");
    assert_eq!(entry.reason, "Japanese spacing differs");
    assert_eq!(entry.issue_id, "raikiri-spike-6qrnr.15.15");
    assert!(expected.get("css/css-text/spacing.html").is_none());
    assert!(expected.get("css/css-text/spacing.html?lang=JA").is_none());
}

#[test]
fn expected_failure_accepts_a_plane_work_item_id() {
    let (expected, errors) = ExpectedFailures::parse(
        "css/foo.html | tracked in Plane | RAIKIRI-21 | 2026-10-11 | 2026-11-11\n",
        "expected-failures.txt",
    );
    assert!(errors.is_empty(), "unexpected parser errors: {errors:?}");
    assert_eq!(expected.entries[0].issue_id, "RAIKIRI-21");
}

#[test]
fn expected_failure_rejects_non_exact_ids_and_invalid_review_metadata() {
    let invalid_rows = [
        "css/foo/* | wildcard | raikiri-spike-1 | 2026-10-04 | 2026-11-04",
        "../foo.html | traversal | raikiri-spike-1 | 2026-10-04 | 2026-11-04",
        "css/foo.html? | empty query | raikiri-spike-1 | 2026-10-04 | 2026-11-04",
        "css/foo.html#fragment | fragment | raikiri-spike-1 | 2026-10-04 | 2026-11-04",
        "css/foo.html | no local issue | fulgur-1 | 2026-10-04 | 2026-11-04",
        "css/foo.html | Plane id without a number | RAIKIRI- | 2026-10-04 | 2026-11-04",
        "css/foo.html | lowercase Plane id | raikiri-21 | 2026-10-04 | 2026-11-04",
        "css/foo.html | bad date | raikiri-spike-1 | 2026-02-30 | 2026-11-04",
        "css/foo.html | review must follow add | raikiri-spike-1 | 2026-10-04 | 2026-10-04",
    ];

    for row in invalid_rows {
        let (expected, errors) = ExpectedFailures::parse(row, "expected-failures.txt");
        assert!(expected.is_empty(), "accepted invalid row: {row}");
        assert_eq!(errors.len(), 1, "row: {row}");
        assert!(
            matches!(errors[0], ExpectError::MalformedLine { .. }),
            "row: {row}, error: {:?}",
            errors[0]
        );
    }
}

#[test]
fn expected_failure_parser_rejects_bad_shapes_and_preserves_review_date_errors() {
    let invalid_rows = [
        "css/foo.html | reason | raikiri-spike-1 | 2026-10-04 | 2026-11-04 ",
        "css/foo.html\treason | raikiri-spike-1 | 2026-10-04 | 2026-11-04",
        "css/foo.html | reason | raikiri-spike-1 | 2026-10-04",
        "css/foo.html |  | raikiri-spike-1 | 2026-10-04 | 2026-11-04",
        "css/foo.html | reason | raikiri-spike-1 | 2026-10-04 | not-a-date",
        "css/foo.html?x?y | reason | raikiri-spike-1 | 2026-10-04 | 2026-11-04",
    ];

    for row in invalid_rows {
        let (expected, errors) = ExpectedFailures::parse(row, "expected-failures.txt");
        assert!(expected.is_empty(), "accepted invalid row: {row:?}");
        assert_eq!(errors.len(), 1, "row: {row:?}");
        assert!(
            matches!(errors[0], ExpectError::MalformedLine { .. }),
            "row: {row:?}, error: {:?}",
            errors[0]
        );
    }
}

#[test]
fn expectation_set_rejects_a_malformed_expected_failure_file() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "tracked-wpt.txt",
        "known-issues.txt",
        "raikiri-baseline.txt",
        "quarantine.txt",
        "deprecated.txt",
    ] {
        std::fs::write(dir.path().join(name), "# header\n").unwrap();
    }
    std::fs::write(
        dir.path().join("expected-failures.txt"),
        "css/foo.html | missing issue id | upstream-1 | 2026-10-04 | 2026-11-04\n",
    )
    .unwrap();

    let error = match ExpectationSet::load_from(dir.path()) {
        Err(error) => error,
        Ok(_) => panic!("malformed expected-failure rows must reject the expectation set"),
    };

    assert!(
        matches!(error, ExpectError::MalformedLine { .. }),
        "unexpected error: {error:?}"
    );
}

#[test]
fn tracked_parses_test_ids_and_skips_comments() {
    let content = "# comment\n\
                       css/css-page/page-margin-boxes-001\n\
                       \n\
                       css/css-fragmentation/break-before-001\n";
    let tracked = TrackedWpt::parse(content, "t.txt").unwrap();
    assert_eq!(
        tracked.entries,
        vec![
            "css/css-page/page-margin-boxes-001".to_owned(),
            "css/css-fragmentation/break-before-001".to_owned(),
        ]
    );
}

#[test]
fn known_issues_parses_pattern_and_reason() {
    let content = "css/css-transitions/* | non-goal, interactive\n\
                       html/interaction/* | non-goal, interactive\n";
    let ki = KnownIssues::parse(content, "k.txt").unwrap();
    assert_eq!(
        ki.entries,
        vec![
            (
                "css/css-transitions/*".to_owned(),
                "non-goal, interactive".to_owned()
            ),
            (
                "html/interaction/*".to_owned(),
                "non-goal, interactive".to_owned()
            ),
        ]
    );
}

#[test]
fn known_issues_rejects_missing_reason() {
    let content = "css/foo | \n";
    let err = KnownIssues::parse(content, "k.txt").unwrap_err();
    match err {
        ExpectError::MalformedLine { line_no, .. } => assert_eq!(line_no, 1),
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn baseline_deduplicates_via_hashset() {
    let content = "css/foo/bar-001\ncss/foo/bar-001\ncss/foo/baz-002\n";
    let base = Baseline::parse(content, "b.txt").unwrap();
    assert_eq!(base.entries.len(), 2);
}

#[test]
fn baseline_rejects_pipe_delimiter() {
    // Row copy-pasted from quarantine.txt should surface as Malformed
    // rather than pass silently as a bogus test id.
    let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n";
    let err = Baseline::parse(content, "raikiri-baseline.txt").unwrap_err();
    match err {
        ExpectError::MalformedLine {
            file,
            line_no,
            reason,
        } => {
            assert_eq!(file, "raikiri-baseline.txt");
            assert_eq!(line_no, 1);
            assert!(reason.contains("pipe delimiter"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn baseline_rejects_trailing_whitespace() {
    let content = "css/foo/bar-001   \n";
    let err = Baseline::parse(content, "raikiri-baseline.txt").unwrap_err();
    match err {
        ExpectError::MalformedLine {
            line_no, reason, ..
        } => {
            assert_eq!(line_no, 1);
            assert!(reason.contains("trailing whitespace"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn deprecated_rejects_pipe_delimiter() {
    let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n";
    let err = Deprecated::parse(content, "deprecated.txt").unwrap_err();
    match err {
        ExpectError::MalformedLine {
            file,
            line_no,
            reason,
        } => {
            assert_eq!(file, "deprecated.txt");
            assert_eq!(line_no, 1);
            assert!(reason.contains("pipe delimiter"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn deprecated_rejects_trailing_whitespace() {
    let content = "css/foo/bar-001\t\n";
    let err = Deprecated::parse(content, "deprecated.txt").unwrap_err();
    match err {
        ExpectError::MalformedLine {
            line_no, reason, ..
        } => {
            assert_eq!(line_no, 1);
            assert!(reason.contains("trailing whitespace"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn baseline_and_deprecated_accept_comment_and_blank_lines() {
    let content = "# header\n\ncss/foo/bar-001\n\n# trailing comment\n";
    let base = Baseline::parse(content, "b.txt").unwrap();
    assert_eq!(base.entries.len(), 1);
    let dep = Deprecated::parse(content, "d.txt").unwrap();
    assert_eq!(dep.entries.len(), 1);
}

#[test]
fn quarantine_parses_8_cols_and_enum_values() {
    let content = "css/css-page/page-margin-boxes-001 | macos | aarch64 | vello_cpu | pixel-exact | Intermittent 1-pixel diff | https://example/issues/123 | 2026-08-01\n";
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(errs.is_empty(), "expected no errors, got {errs:?}");
    assert_eq!(q.entries.len(), 1);
    let e = &q.entries[0];
    assert_eq!(e.test_id, "css/css-page/page-margin-boxes-001");
    assert_eq!(e.platform, PlatformFilter::MacOs);
    assert_eq!(e.arch, ArchFilter::Aarch64);
    assert_eq!(e.renderer, RendererFilter::VelloCpu);
    assert_eq!(e.tolerance, ToleranceFilter::PixelExact);
    assert_eq!(e.reason, "Intermittent 1-pixel diff");
    assert_eq!(e.issue_link, "https://example/issues/123");
    assert_eq!(e.added_date, time::macros::date!(2026 - 08 - 01));
}

#[test]
fn quarantine_accepts_wildcards() {
    let content = "css/foo | * | * | * | * | reason | issue | 2026-08-05\n";
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(errs.is_empty(), "expected no errors, got {errs:?}");
    let e = &q.entries[0];
    assert_eq!(e.platform, PlatformFilter::Any);
    assert_eq!(e.arch, ArchFilter::Any);
    assert_eq!(e.renderer, RendererFilter::Any);
    assert_eq!(e.tolerance, ToleranceFilter::Any);
}

#[test]
fn quarantine_rejects_wrong_col_count() {
    let content = "css/foo | linux | x86_64\n"; // 3 cols
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(q.entries.is_empty());
    assert_eq!(errs.len(), 1);
    match &errs[0] {
        ExpectError::MalformedLine {
            line_no, reason, ..
        } => {
            assert_eq!(*line_no, 1);
            assert!(reason.contains("expected 8"));
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn quarantine_rejects_unknown_platform() {
    let content = "css/foo | plan9 | x86_64 | vello_cpu | low | r | i | 2026-08-01\n";
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(q.entries.is_empty());
    assert_eq!(errs.len(), 1);
    match &errs[0] {
        ExpectError::UnknownEnum { field, value, .. } => {
            assert_eq!(*field, "platform");
            assert_eq!(value, "plan9");
        }
        other => panic!("expected UnknownEnum, got {other:?}"),
    }
}

#[test]
fn quarantine_rejects_malformed_added_date() {
    let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | not-a-date\n";
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(q.entries.is_empty());
    assert_eq!(errs.len(), 1);
    match &errs[0] {
        ExpectError::MalformedLine {
            line_no, reason, ..
        } => {
            assert_eq!(*line_no, 1);
            assert!(reason.contains("added_date"), "got: {reason}");
            assert!(reason.contains("not-a-date"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn quarantine_rejects_calendar_invalid_added_date_feb30() {
    // 2026-02-30 is syntactically YYYY-MM-DD but no such calendar day exists.
    let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-02-30\n";
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(q.entries.is_empty());
    assert_eq!(errs.len(), 1);
    match &errs[0] {
        ExpectError::MalformedLine {
            line_no, reason, ..
        } => {
            assert_eq!(*line_no, 1);
            assert!(reason.contains("added_date"), "got: {reason}");
            assert!(reason.contains("2026-02-30"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn quarantine_rejects_calendar_invalid_added_date_month13() {
    // Month 13 is syntactically valid but out of range.
    let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-13-01\n";
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(q.entries.is_empty());
    assert_eq!(errs.len(), 1);
    match &errs[0] {
        ExpectError::MalformedLine {
            line_no, reason, ..
        } => {
            assert_eq!(*line_no, 1);
            assert!(reason.contains("added_date"), "got: {reason}");
            assert!(reason.contains("2026-13-01"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn quarantine_accepts_leap_year_feb_29() {
    // 2024 is a leap year, so 2024-02-29 is a valid calendar day.
    let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2024-02-29\n";
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(errs.is_empty(), "expected no errors, got {errs:?}");
    assert_eq!(q.entries.len(), 1);
    assert_eq!(q.entries[0].added_date, time::macros::date!(2024 - 02 - 29));
}

#[test]
fn quarantine_rejects_trailing_whitespace() {
    // Trailing whitespace on a data row surfaces as MalformedLine
    // (spec §12.10) even when the column count is right.
    let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01   \n";
    let (q, errs) = Quarantine::parse(content, "q.txt");
    assert!(
        q.entries.is_empty(),
        "expected no entries, got {:?}",
        q.entries
    );
    assert_eq!(errs.len(), 1);
    match &errs[0] {
        ExpectError::MalformedLine {
            line_no, reason, ..
        } => {
            assert_eq!(*line_no, 1);
            assert!(reason.contains("trailing whitespace"), "got: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn quarantine_accumulates_multiple_row_errors() {
    // 3 rows: valid, malformed (wrong column count), valid.
    // Expect the malformed row to accumulate as an error while both
    // valid rows survive as entries.
    let content = "\
css/a | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01
css/bad | linux | x86_64
css/b | macos | aarch64 | skia | high | r | i | 2026-08-02
";
    let (q, errors) = Quarantine::parse(content, "q.txt");
    assert_eq!(
        q.entries.len(),
        2,
        "expected 2 valid entries, got {:?}",
        q.entries
    );
    assert_eq!(q.entries[0].test_id, "css/a");
    assert_eq!(q.entries[0].line_no, 1);
    assert_eq!(q.entries[1].test_id, "css/b");
    assert_eq!(q.entries[1].line_no, 3);
    assert_eq!(errors.len(), 1);
    match &errors[0] {
        ExpectError::MalformedLine {
            line_no, reason, ..
        } => {
            assert_eq!(*line_no, 2);
            assert!(reason.contains("expected 8"), "got reason: {reason}");
        }
        other => panic!("expected MalformedLine, got {other:?}"),
    }
}

#[test]
fn quarantine_continues_after_enum_error() {
    // Bad platform followed by a valid row. Ensure the valid row is
    // still surfaced in entries and the enum error is captured.
    let content = "\
css/bad | plan9 | x86_64 | vello_cpu | low | r | i | 2026-08-01
css/ok | macos | aarch64 | skia | high | r | i | 2026-08-02
";
    let (q, errors) = Quarantine::parse(content, "q.txt");
    assert_eq!(q.entries.len(), 1);
    assert_eq!(q.entries[0].test_id, "css/ok");
    assert_eq!(errors.len(), 1);
    match &errors[0] {
        ExpectError::UnknownEnum {
            field,
            value,
            line_no,
            ..
        } => {
            assert_eq!(*field, "platform");
            assert_eq!(value, "plan9");
            assert_eq!(*line_no, 1);
        }
        other => panic!("expected UnknownEnum, got {other:?}"),
    }
}

#[test]
fn quarantine_records_only_first_column_error_per_row() {
    // A row with two invalid columns (bad platform AND bad arch) must
    // produce exactly one error — the first-failing column (platform) —
    // per the short-circuit-per-row contract. If a future refactor
    // switches to intra-row accumulation this test will fail loudly.
    let content = "css/x | plan9 | notarch | vello_cpu | low | r | i | 2026-08-01\n";
    let (q, errors) = Quarantine::parse(content, "q.txt");
    assert!(q.entries.is_empty());
    assert_eq!(errors.len(), 1);
    match &errors[0] {
        ExpectError::UnknownEnum { field, value, .. } => {
            assert_eq!(*field, "platform");
            assert_eq!(value, "plan9");
        }
        other => panic!("expected UnknownEnum(platform), got {other:?}"),
    }
}

#[test]
fn expect_error_display_and_source_chain() {
    // Io variant delegates source to the wrapped io::Error.
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
    let err: ExpectError = io_err.into();
    assert!(std::error::Error::source(&err).is_some());
    assert!(format!("{err}").contains("missing"));

    let err = ExpectError::MalformedLine {
        file: "q.txt".to_owned(),
        line_no: 42,
        reason: "bad".to_owned(),
    };
    assert!(std::error::Error::source(&err).is_none());
    assert_eq!(format!("{err}"), "q.txt:42: malformed line (bad)");
}

#[test]
fn load_from_workspace_root_reads_the_header_only_files() {
    // Integration-style: verify each expectations/*.txt against the
    // full expected content (spec §12.9 tracked categories + §2 Non-Goals
    // waivers). Count-only assertions cannot detect typos, duplicates, or
    // ordering drift — assert exact membership to make transcription bugs
    // observable.
    let set = ExpectationSet::load_from_workspace_root()
        .expect("workspace expectations/ should be present");

    let tracked: Vec<&str> = set.tracked.entries.iter().map(String::as_str).collect();
    assert_eq!(
        tracked,
        vec![
            // P1 — Foundation
            "css/css-fonts/",
            "css/css-color/",
            "css/css-backgrounds/",
            "css/css-images/",
            "css/css-values/",
            "css/css-text/",
            "css/css-text-decor/",
            "css/css-writing-modes/",
            "css/selectors/",
            "html/rendering/",
            // P2 — Layout primitives
            "css/css-tables/",
            "css/css-grid/",
            "css/css-flexbox/",
            // P3 — GCPM / paged media
            "css/css-page/",
            "css/css-break/",
            // P4 — Low priority
            "css/css-transforms/",
        ],
    );

    let known_issues: Vec<(&str, &str)> = set
        .known_issues
        .entries
        .iter()
        .map(|(pat, reason)| (pat.as_str(), reason.as_str()))
        .collect();
    // Check that all expected baseline entries are present (allow additional quarantines)
    let expected_baseline = vec![
        (
            "css/css-animations/",
            "Non-goal (interactive, §2 Non-Goals + §12.9)",
        ),
        (
            "css/css-transitions/",
            "Non-goal (interactive, §2 Non-Goals + §12.9)",
        ),
        (
            "html/interaction/",
            "Non-goal (interactive rendering, §2 Non-Goals + §12.9)",
        ),
        (
            "css/css-ruby/",
            "Non-goal for MVP (JIS X 4051 / 縦書き outside MVP scope, §2 Non-Goals)",
        ),
        (
            "accname/",
            "Non-goal (Consumer builds a11y tree from hints, §2 Non-Goals)",
        ),
        (
            "wai-aria/",
            "Non-goal (Consumer builds a11y tree from hints, §2 Non-Goals)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-blend-mode.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-descendants.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-ellipsis.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-flex.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-fragmentation.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-inline-block-child.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-inline.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-multiline-background-image.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-multiline-linebreak.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-on-body-not-propagated-to-root.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-on-body-scroll.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-out-of-flow-child.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-relative-child.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-scaled.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-stacking-context-child.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-text-align.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-text-decorations.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-text-text-emphasis.html",
            "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-border-area-border-on-top.html",
            "background-clip multi-layer (border-area, border-box) with background-image list not implemented (single-layer scope carving)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-border-area-multiple-backgrounds.html",
            "background-clip multi-layer (border-area, border-box, content-box) with background-image list not implemented",
        ),
        (
            "css/css-backgrounds/background-clip/clip-border-area-text.html",
            "background-clip border-area text union (CSS Backgrounds 4 §2.6) requires glyph + border-area compositing not implemented",
        ),
        (
            "css/css-backgrounds/background-clip/clip-border-area-border-shape-background-position.html",
            "border-shape inset(0) background-position interaction not implemented (CSS Borders 4)",
        ),
        (
            "css/css-backgrounds/background-clip/clip-border-shape-table-part-background.html",
            "table-part background with border-shape not implemented",
        ),
    ];
    // Allow additional entries beyond baseline (goal B quarantines)
    for exp in &expected_baseline {
        assert!(
            known_issues.contains(exp),
            "expected known-issue {:?} missing",
            exp
        );
    }

    // The baseline count is derived from the file instead of pinned, so
    // parallel changes to the baseline do not conflict on one number. A
    // duplicated id would collapse in the set and lower `entries.len()`
    // below the number of data lines, so equality also rejects duplicates.
    let baseline_path = workspace_expectations_dir().join("raikiri-baseline.txt");
    let baseline_content =
        std::fs::read_to_string(&baseline_path).expect("raikiri-baseline.txt should be readable");
    let baseline_lines = iter_data_lines(&baseline_content).count();
    assert_eq!(
        set.baseline.entries.len(),
        baseline_lines,
        "raikiri-baseline.txt has duplicate test ids"
    );
    // Floor against an accidental truncation or emptying of the file.
    assert!(
        baseline_lines >= 4000,
        "raikiri-baseline.txt shrank to {baseline_lines} entries"
    );
    for id in [
        "css/css-pseudo/first-letter-004.html",
        "css/css-pseudo/first-letter-005.html",
        "css/css-pseudo/first-letter-with-before-after.html",
    ] {
        assert!(set.baseline.entries.contains(id));
    }
    assert!(
        !set.baseline
            .entries
            .contains("css/css-pseudo/first-letter-003.html")
    );
    assert!(
        !set.baseline
            .entries
            .contains("css/css-backgrounds/first-letter-space-not-selected.html")
    );

    // + CSS Break between-box page/column avoidance exact slice (+15).
    for index in 0..15 {
        let id = format!("css/css-break/break-between-avoid-{index:03}.html");
        assert!(set.baseline.entries.contains(id.as_str()));
    }

    assert!(
        set.baseline
            .entries
            .contains("css/css-overflow/text-overflow-ellipsis-002.html")
    );
    for id in [
        "css/CSS2/tables/caption-side-applies-to-006.xht",
        "css/CSS2/tables/empty-cells-applies-to-014.xht",
        "css/CSS2/tables/table-vertical-align-baseline-007.xht",
        "css/css-color/t44-currentcolor-background-b.xht",
        "css/css-color/color-mix-currentcolor-001.html",
        "css/css-color/color-mix-currentcolor-002.html",
        "css/css-color/color-mix-currentcolor-003.html",
        "css/css-color/color-mix-currentcolor-nested-for-color-property.html",
        "css/css-lists/list-style-type-string-001a.html",
        "css/css-lists/list-style-type-string-001b.html",
        "css/css-lists/content-property/marker-text-matches-decimal.html",
        "css/css-lists/content-property/marker-text-matches-decimal-leading-zero.html",
        "css/css-lists/content-property/marker-text-matches-lower-latin.html",
        "css/css-lists/content-property/marker-text-matches-upper-latin.html",
        "css/css-lists/content-property/marker-text-matches-lower-roman.html",
        "css/css-lists/content-property/marker-text-matches-upper-roman.html",
    ] {
        assert!(set.baseline.entries.contains(id));
    }
    assert!(
        set.baseline
            .entries
            .contains("css/CSS2/linebox/vertical-align-007.xht")
    );
    assert!(
        set.baseline
            .entries
            .contains("css/css-ui/text-overflow-ellipsis-indent-001.html")
    );
    for id in [
        "css/mediaqueries/mq-range-001.html",
        "css/mediaqueries/mq-invalid-media-type-layer-001.html",
        "css/mediaqueries/relative-units-001.html",
        "css/CSS2/positioning/position-relative-004.xht",
        "css/CSS2/positioning/position-relative-038.xht",
    ] {
        assert!(set.baseline.entries.contains(id), "missing WPT pin: {id}");
    }
    assert!(set.quarantine.is_empty());
    assert!(set.deprecated.is_empty());
}
