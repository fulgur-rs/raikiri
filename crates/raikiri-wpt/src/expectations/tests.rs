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

    // baseline populated via chore(wpt): check PASS file-level parsing tests + goal G (79) + selectors (14) (WPT 97ea26e)
    // + grid reftest H (+20) + text reftest retry2 J2 (+176) + flexbox reftest retry2 I2 (+191)
    // + css-tables reftest range (+29) + table-layout/border-collapse parsing-valid (+2)
    // + css-tables parsing-8 (+7: border-spacing/caption-side/empty-cells valid + caption-side/empty-cells/table-layout/border-collapse computed)
    // + css-tables reftest tables-range (+4: zero-rowspan-001, row-group-order, percent-height-replaced-in-percent-cell.tentative, table-cell-baseline-static-position).
    // - min-height-table.html (4054 pixels) and min-height-table-2.html
    //   (484054 pixels) (-2, baseline migration: merged content-box min/max
    //   sizing behavior remains unpinned pending the table-height tranche).
    // + css-text-decor parsing jqg8 (+10)
    // + page-valid + flex-basis-valid + flex-invalid parsing tgag (+3).
    // + segment-break removable/ignorable exact reftests (+5).
    // + text-transform capitalize whitespace/abspos exact reftest (+1).
    // - hyphens-punctuation-001 (-1, 9q1p: container-width re-break exposed
    //   a false pass; genuine pass needs hyphenation dictionaries).
    // + text-align end-001..008 (sans 009/010) + start-001..008 + start-010
    //   (+17, dir-attribute UA rules: explicit-direction and dir=ltr/rtl/auto
    //   cases now resolve; 009/010 need zero-width RLM handling, follow-up).
    // - bidi-tab-001 (-1: dir rules made direction real, exposing
    //   direction-naive tab-stop expansion in RTL spans; needs bidi-aware
    //   tab stops, follow-up).
    // + subpixel-table-cell-width-001/002 (+2, glk7 grid text-skip:
    //   non-element children carry no grid structure).
    // + segment-break-transformation-ignorable/removable x5 + text-transform-capitalize-034
    //   (+6, css-text-whitespace #10: multi-node runs, ignorable neighbors, abspos inline).
    // + CSS Text PASS-only WPT ranges (+25: white-space/text-wrap,
    //   word-break/overflow-wrap, text-indent, tab-size).
    // + text-align-last paint-time final-line alignment (+16).
    // + letter-spacing bridge and hyphens PASS-only range (+6).
    // + text-transform combinations, full-size-kana, and language tailoring
    //   PASS-only range (+54).
    // + css-color invalid parsing probe (9 files, WPT 97ea26e).
    // + css-backgrounds reftest range (30 new current passes, 8 stale inherited
    //   pins removed; 400x300 EXACT, WPT 97ea26e, net +22).
    // quarantine and deprecated stay empty until a developer PR adds a flake or crasher.

    // + flexbox_fbfc (+1, y0zo initial slice: exact 800x600 pass).
    // + flexbox-overflow-vert-003 (+1, y0zo overflow clip: exact 800x600 pass).
    // + flexbox gap/wrapping slice (+8, exact 800x600 pass).
    // + grid item-sizing slice (+2, exact 800x600 pass).
    // + table flex percentage width (+1, exact 800x600 pass).
    // + abspos static-position slice (+9, exact 800x600 pass).
    // + abspos static-position follow-up (+7, exact 800x600 pass).
    // - inline-flex percentage sizing: one flex-item check restored after
    //   the Taffy calc resolver was connected; one remains deferred.
    // + white-space break-spaces narrow-container slice (+2, exact 800x600 pass).
    // + word-break auto-phrase baseline slice (+2, exact 800x600 pass).
    // + line-breaking segment-break slice (+3, exact 800x600 pass).
    // + word-spacing `ch` shaping slice (+1, exact 800x600 pass).
    // + white-space follow-up slice (+6, exact 800x600 pass).
    // + CSS Lists marker-position foundation slice (+1, exact static reftest).
    // + y0zo gap-008-ltr/gap-009-ltr (+2, exact 800x600 passes).
    // + CSS Text control-chars-000 literal generated-content slice (+1).
    // + CSS generated-content counter, attr fallback, quote, and image slices (+5).
    // + CSS generated-content counters() nested-scope slice (+1).
    // + CSS generated-content before/after literal string slice (+1).
    // + CSS Text overflow-wrap cluster and min-content slice (+3).
    // + CSS Text letter-spacing ligatures slice (+1).
    // + CSS Text writing-system font slice (+1).
    // + CSS Text text-group-align/text-spacing-trim/hanging-punctuation slice (+28).
    // + CSS Text shaping slice (+17).
    // + CSS Text text-autospace PASS-only slice (+2).
    // + CSS Images gradients and CSS Page image-resource slices (+6).
    // + CSS Page :left/:right spread-pseudo content-width slice (+1).
    // + CSS Page named-page column-flex propagation slice (+2).
    // + CSS Tables absolute-positioned auto-width slice (+2).
    // + CSS Tables visibility-hidden collapsed-border slice (+1).
    // + CSS Break forced page-break reftest (+1).
    // + CSS Break Flexbox print Fulgur v0.40.0 PASS slice (+7).
    // + CSS Break Flexbox additional Fulgur v0.40.0 PASS slice (+7).
    // + CSS Break logical min-block-size Fulgur v0.40.0 PASS slice (+2).
    // + CSS Multi-column nested block-flow exact slice (+4).
    // + CSS Text white-space/line-breaking resource exact slice (+5).
    // + CSS Text tab-size spacing-002/003 exact PASS slice (+2).
    // + CSS Values exact calc nesting/rounding slice (+5).
    // + CSS Selectors exact structural-selector slice (+23).
    // + CSS Pseudo exact active-selection/first-letter slice (+2).
    // + CSS Transforms exact 2D transform-origin/skew slice (+5).
    // + CSS Borders exact current-color and sub-unit slice (+2).
    // + CSS Text Decoration inset endpoint smoke (+1).
    // + CSS Text Decoration fixed underline-offset horizontal smoke (+1).
    // + CSS Inline block-in-inline margin-collapse smoke (+1).
    // + CSS Text overflow-wrap continuation across inline spans (+2).
    // + CSS Tables body stylesheet/subpixel padding exact slice (+1).
    // + CSS Page out-of-flow named-page exact slice (+2).
    // - Remove stale CSS Grid percentage-size-subitems-001 PASS pin (-1).
    // + CSS Page hidden-child named-page boundary exact slice (+1).
    // + CSS Page inline canvas named-page boundary exact slice (+2).
    // + CSS Page nested named-page propagation exact slice (+2).
    // + CSS Text text-indent exact 800x600 review-approved slice (+3).
    // + CSS Text negative word-spacing exact 800x600 review-approved pair (+1).
    // + CSS Text line-break:anywhere first-stage exact WPT PASS slice (+21).
    // + CSS Inline anonymous inline/baseline PASS slice (+2).
    // + CSS Text Decoration percentage underline-offset exact slice (+1).
    // - Drop CSS Text shaping/autospace pins that do not pass exactly (-4).
    // + CSS Text text-autospace vs/zh exact resource slice (+2).
    // + CSS Inline collapsed whitespace between inline boxes (+1).
    // + CSS Flexbox order-painting exact 800x600 pair (+1).
    // + CSS Page basic-pagination exact 800x600 slice (+5).
    // + CSS Page monolithic-overflow exact 800x600 slice (+5).
    // + CSS Break widows-orphans paged-text exact 800x600 slice (+11).
    // + CSS Page fixedpos exact 800x600 slice (+9).
    // + css/printing fragmented-inline-block exact 800x600 slice (+2).
    // + CSS Text letter-spacing preserved final newline exact slice (+1).
    // + CSS Text text-autospace inline-element exact slice (+2).
    // + CSS Text text-autospace font-backed Latin adjacency exact slice (+2).
    // + CSS Text font-resolved Arabic ZWJ join exact slice (+2).
    // + CSS Text writing-system script-tag casing exact slice (+1).
    // + CSS Text basic overflow-wrap break-word exact slice (+1).
    // + CSS Text nowrap suppresses overflow-wrap exact slice (+1).
    // + CSS Text text-justify none exact slice (+1).
    // + CSS Text word-space-transform:space ZWSP/wbr exact slice (+1).
    // + CSS Text word-space-transform:ideographic-space ZWSP/wbr exact slice (+1).
    // + CSS Text word-space-transform:none inline/wbr opt-out exact slice (+2).
    // + CSS Text word-space-transform:space inline enable exact slice (+1).
    // + CSS Text no virtual boundaries without auto-phrase exact slice (+1).
    // + CSS Text no-autospace vs normal exact match+mismatch slice (+1).
    // + CSS Text ideograph-numeric text-autospace Ahem exact slice (+1).
    // + CSS Text supplementary ideograph text-autospace exact slice (+1).
    assert_eq!(set.baseline.entries.len(), 1355);
    assert!(set.quarantine.is_empty());
    assert!(set.deprecated.is_empty());
}
