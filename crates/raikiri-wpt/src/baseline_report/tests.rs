use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn row(id: &str, status: Status) -> Row {
    Row {
        id: id.to_owned(),
        status,
        detail: String::new(),
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn baseline_ids_skip_comments_and_blank_lines() {
    let text = "# header\n\ncss/a/one.html\n  css/b/two.html  \n# trailing\n";
    assert_eq!(
        parse_baseline_ids(text),
        vec!["css/a/one.html", "css/b/two.html"]
    );
}

#[test]
fn only_css_parsing_pages_use_the_testharness_runner() {
    assert!(is_parsing_id("css/css-align/parsing/grid-gap-invalid.html"));
    assert!(!is_parsing_id(
        "css/css-text/text-autospace/text-autospace-001.html"
    ));
    assert!(!is_parsing_id("html/parsing/foo.html"));
}

#[test]
fn status_names_round_trip() {
    for status in [
        Status::Pass,
        Status::Fail,
        Status::XFail,
        Status::XPass,
        Status::Error,
        Status::Skip,
    ] {
        assert_eq!(Status::parse(status.as_str()), Some(status));
    }
    assert_eq!(Status::parse("MAYBE"), None);
}

#[test]
fn tsv_round_trips_and_flattens_detail_whitespace() {
    let rows = vec![
        row("a", Status::Pass),
        Row {
            id: "b".to_owned(),
            status: Status::Fail,
            detail: "line one\nline\ttwo".to_owned(),
        },
    ];
    let text = write_tsv(&rows);
    assert_eq!(text, "a\tPASS\t\nb\tFAIL\tline one line two\n");
    let parsed = parse_tsv(&text).expect("parse");
    assert_eq!(parsed[0], rows[0]);
    assert_eq!(parsed[1].detail, "line one line two");
}

#[test]
fn tsv_rejects_a_line_with_an_unknown_status() {
    let error = parse_tsv("a\tPASS\t\nb\tNOPE\t\n").expect_err("unknown status");
    assert!(error.contains("line 2"), "{error}");
}

#[test]
fn diff_reports_only_pass_transitions() {
    let before = vec![
        row("keep", Status::Pass),
        row("lost", Status::Pass),
        row("gained", Status::Fail),
        row("gone", Status::Pass),
        row("still-bad", Status::Fail),
    ];
    let after = vec![
        row("keep", Status::Pass),
        row("lost", Status::Error),
        row("gained", Status::Pass),
        row("still-bad", Status::Fail),
        row("new", Status::Pass),
    ];
    let result = diff(&before, &after);
    assert_eq!(result.regressions, vec!["lost"]);
    assert_eq!(result.fixes, vec!["gained"]);
    assert_eq!(result.missing, vec!["gone"]);
}

#[test]
fn diff_renders_counts_then_ids() {
    let result = Diff {
        regressions: strings(&["a", "b"]),
        fixes: Vec::new(),
        missing: strings(&["c"]),
        weakened: strings(&["d"]),
    };
    assert_eq!(
        render_diff(&result),
        "regressions: 2\n  a\n  b\nfixes: 0\nmissing: 1\n  c\nweakened: 1\n  d\n"
    );
}

#[test]
fn parallel_runs_keep_input_order_and_report_each_row_once() {
    let ids = strings(&["slow", "fast", "mid"]);
    let seen = AtomicUsize::new(0);
    let rows = run_ids_with(
        &ids,
        3,
        |id| {
            if id == "slow" {
                std::thread::sleep(Duration::from_millis(60));
            }
            row(id, Status::Pass)
        },
        &|_| {
            seen.fetch_add(1, Ordering::SeqCst);
        },
    );
    let order: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(order, ["slow", "fast", "mid"]);
    assert_eq!(seen.load(Ordering::SeqCst), 3);
}

#[test]
fn a_panicking_runner_becomes_an_error_row() {
    let row = catch_row("boom", || panic!("kaboom"));
    assert_eq!(row.status, Status::Error);
    assert!(row.detail.contains("kaboom"), "{}", row.detail);
}

fn pair(kind: ReftestKind, status: Status, detail: &str) -> (ReftestKind, Status, String) {
    (kind, status, detail.to_owned())
}

#[test]
fn any_matching_reference_is_enough() {
    let results = vec![
        pair(ReftestKind::Match, Status::Fail, "first differs"),
        pair(ReftestKind::Match, Status::Pass, ""),
    ];
    assert_eq!(combine_pairs(results), (Status::Pass, String::new()));
}

#[test]
fn every_mismatch_reference_is_required() {
    let results = vec![
        pair(ReftestKind::Mismatch, Status::Pass, ""),
        pair(ReftestKind::Mismatch, Status::Fail, "identical"),
    ];
    assert_eq!(
        combine_pairs(results),
        (Status::Fail, "identical".to_owned())
    );
}

#[test]
fn a_passing_match_does_not_excuse_a_failing_mismatch() {
    let results = vec![
        pair(ReftestKind::Match, Status::Pass, ""),
        pair(ReftestKind::Mismatch, Status::Fail, "identical"),
    ];
    assert_eq!(combine_pairs(results).0, Status::Fail);
}

#[test]
fn without_a_passing_alternative_the_first_problem_is_reported() {
    let results = vec![
        pair(ReftestKind::Match, Status::Error, "run: boom"),
        pair(ReftestKind::Match, Status::Fail, "differs"),
    ];
    assert_eq!(
        combine_pairs(results),
        (Status::Error, "run: boom".to_owned())
    );
}

#[test]
fn report_args_parse_flags_and_repeatable_only() {
    let args = strings(&[
        "--wpt-root",
        "w",
        "--baseline",
        "b.txt",
        "--output",
        "o.tsv",
        "--jobs",
        "4",
        "--only",
        "x.html",
        "--only",
        "y.html",
        "--limit",
        "2",
    ]);
    let options = parse_report_args(&args).expect("parse");
    assert_eq!(options.wpt_root, PathBuf::from("w"));
    assert_eq!(options.baseline, PathBuf::from("b.txt"));
    assert_eq!(options.output, Some(PathBuf::from("o.tsv")));
    assert_eq!(options.jobs, 4);
    assert_eq!(options.only, vec!["x.html", "y.html"]);
    assert_eq!(options.limit, Some(2));
}

#[test]
fn expected_failure_classification_only_accepts_assertion_and_pixel_failures() {
    let expected = ExpectedFailure {
        test_id: "css/foo.html?variant=ja".to_owned(),
        reason: "Japanese glyph spacing differs".to_owned(),
        issue_id: "raikiri-spike-6qrnr.15.15".to_owned(),
        added_date: time::macros::date!(2026 - 10 - 04),
        review_by: time::macros::date!(2026 - 11 - 04),
        line_no: 2,
    };

    let (status, detail) =
        classify_expected_failure(Status::Fail, "pixel mismatch".to_owned(), &expected);
    assert_eq!(status, Status::XFail);
    assert!(detail.contains("pixel mismatch"));
    assert!(detail.contains(&expected.reason));
    assert!(detail.contains(&expected.issue_id));

    let (status, detail) = classify_expected_failure(Status::Pass, String::new(), &expected);
    assert_eq!(status, Status::XPass);
    assert!(detail.contains("unexpected pass"));
    assert!(detail.contains(&expected.issue_id));

    for status in [Status::Error, Status::Skip, Status::XFail, Status::XPass] {
        let (actual, detail) =
            classify_expected_failure(status, "must remain visible".to_owned(), &expected);
        assert_eq!(actual, status);
        assert_eq!(detail, "must remain visible");
    }
}

#[test]
fn report_maps_runner_xfail_and_xpass_outcomes_to_distinct_statuses() {
    let xfail = ReftestResult {
        pair_test_id: "css/foo.html?variant=ja".to_owned(),
        outcome: TestOutcome::XFail {
            expected: "font spacing differs".to_owned(),
            actual: "pixel mismatch".to_owned(),
            issue_id: "raikiri-spike-6qrnr.15.15".to_owned(),
        },
        mismatched_pixels: 1,
        total_pixels: 1,
    };
    let (status, detail) = outcome_status(Ok(xfail));
    assert_eq!(status, Status::XFail);
    assert!(detail.contains("font spacing differs"));
    assert!(detail.contains("pixel mismatch"));

    let xpass = ReftestResult {
        pair_test_id: "css/foo.html?variant=ja".to_owned(),
        outcome: TestOutcome::XPass {
            expected: "font spacing differs".to_owned(),
            issue_id: "raikiri-spike-6qrnr.15.15".to_owned(),
        },
        mismatched_pixels: 0,
        total_pixels: 1,
    };
    let (status, detail) = outcome_status(Ok(xpass));
    assert_eq!(status, Status::XPass);
    assert!(detail.contains("unexpected pass"));

    let (status, detail) = outcome_status(Err(ReftestError::Io {
        path: PathBuf::from("missing.html"),
        source: std::io::Error::other("missing"),
    }));
    assert_eq!(status, Status::Error);
    assert!(detail.contains("missing.html"));
}

#[test]
fn expected_failure_report_runs_only_the_exact_query_variant() {
    let directory = tempfile::tempdir().unwrap();
    let wpt_root = directory.path();
    let test_dir = wpt_root.join("css/example");
    std::fs::create_dir_all(&test_dir).unwrap();

    let test = test_dir.join("test.html");
    let reference = test_dir.join("reference.html");
    let variant_html = "<!DOCTYPE html><html><head><link rel='match' href='reference.html'></head><body style='margin:0'><script>document.body.style.background = location.search === '?mode=red' ? 'red' : 'green';</script></body></html>";
    let green_html = "<!DOCTYPE html><html><body style='margin:0;background:green'></body></html>";
    std::fs::write(&test, variant_html).unwrap();
    std::fs::write(&reference, green_html).unwrap();

    let config = ReftestConfig {
        width: 16,
        height: 16,
        ..ReftestConfig::default()
    };

    let (status, _) =
        reftest_status_with_config(wpt_root, "css/example/test.html?mode=red", config);
    assert_eq!(status, Status::Fail, "the declared query should select red");
    let (status, _) = reftest_status_with_config(wpt_root, "css/example/test.html", config);
    assert_eq!(
        status,
        Status::Pass,
        "the undeclared base page selects green"
    );

    let repaired_variant_html = variant_html.replace("? 'red' : 'green';", "? 'green' : 'green';");
    std::fs::write(&test, repaired_variant_html).unwrap();
    let (status, _) =
        reftest_status_with_config(wpt_root, "css/example/test.html?mode=red", config);
    assert_eq!(status, Status::Pass, "the repaired declared variant passes");
}

#[test]
fn an_expected_failure_does_not_convert_a_parsing_skip_to_xfail() {
    let directory = tempfile::tempdir().unwrap();
    let id = "css/css-text/parsing/no-assertions.html";
    let path = directory.path().join(id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "<!doctype html><title>No parsing assertions</title>").unwrap();
    let expected_failures = ExpectedFailures {
        entries: vec![ExpectedFailure {
            test_id: id.to_owned(),
            reason: "the parsing assertion fails".to_owned(),
            issue_id: "raikiri-spike-6qrnr.15.15".to_owned(),
            added_date: time::macros::date!(2026 - 10 - 04),
            review_by: time::macros::date!(2026 - 11 - 04),
            line_no: 1,
        }],
    };

    let row = run_id_with_expected_failures(directory.path(), id, &expected_failures);

    assert_eq!(row.status, Status::Skip);
    assert!(row.detail.contains("no parsing test calls"));
}

#[test]
fn an_expected_failure_does_not_convert_runner_errors_to_xfail() {
    let directory = tempfile::tempdir().unwrap();
    let id = "css/css-text/parsing/missing.html?variant=ja";
    let expected_failures = ExpectedFailures {
        entries: vec![ExpectedFailure {
            test_id: id.to_owned(),
            reason: "variant fails".to_owned(),
            issue_id: "raikiri-spike-6qrnr.15.15".to_owned(),
            added_date: time::macros::date!(2026 - 10 - 04),
            review_by: time::macros::date!(2026 - 11 - 04),
            line_no: 1,
        }],
    };

    let row = run_id_with_expected_failures(directory.path(), id, &expected_failures);

    assert_eq!(row.status, Status::Error);
    assert!(row.detail.contains("query variants for parsing tests"));
}

#[test]
fn report_args_reject_bad_input() {
    assert!(parse_report_args(&strings(&["--nope"])).is_err());
    assert!(parse_report_args(&strings(&["--jobs"])).is_err());
    assert!(parse_report_args(&strings(&["--jobs", "0"])).is_err());
    assert!(parse_report_args(&strings(&["--limit", "many"])).is_err());
}

#[test]
fn report_args_enable_strict_mode_without_changing_the_default() {
    assert!(!parse_report_args(&[]).expect("default args").strict);
    assert!(
        parse_report_args(&strings(&["--strict"]))
            .expect("strict flag")
            .strict
    );
}

#[test]
fn boolean_report_flags_preserve_value_flags_in_any_order() {
    let values = strings(&[
        "--wpt-root",
        "fixtures",
        "--baseline",
        "pass.txt",
        "--output",
        "rows.tsv",
        "--only",
        "a.html",
        "--only",
        "b.html",
        "--jobs",
        "3",
        "--limit",
        "7",
    ]);
    for position in (0..=values.len()).step_by(2) {
        for flags in [["--strict", "--wpt-fonts"], ["--wpt-fonts", "--strict"]] {
            let mut args = values.clone();
            args.splice(position..position, strings(&flags));
            let options = parse_report_args(&args).expect("parse mixed report flags");
            assert!(options.strict);
            assert_eq!(options.wpt_root, PathBuf::from("fixtures"));
            assert_eq!(options.baseline, PathBuf::from("pass.txt"));
            assert_eq!(options.output, Some(PathBuf::from("rows.tsv")));
            assert_eq!(options.only, ["a.html", "b.html"]);
            assert_eq!(options.jobs, 3);
            assert_eq!(options.limit, Some(7));
        }
    }
}

#[test]
fn strict_does_not_hide_a_missing_flag_value() {
    for flag in [
        "--wpt-root",
        "--baseline",
        "--output",
        "--only",
        "--jobs",
        "--limit",
    ] {
        assert_eq!(
            parse_report_args(&strings(&["--strict", flag])).unwrap_err(),
            format!("{flag} requires a value")
        );
    }
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn a_known_passing_reftest_reports_pass() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let row = run_id(
        &root,
        "css/css-text/text-autospace/text-autospace-vertical-upright-001.html",
    );
    assert_eq!(row.status, Status::Pass, "{}", row.detail);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn a_known_passing_parsing_page_reports_pass() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let row = run_id(&root, "css/css-align/parsing/grid-gap-invalid.html");
    assert_eq!(row.status, Status::Pass, "{}", row.detail);
}

#[test]
fn a_missing_test_file_is_an_error_not_a_pass() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt-does-not-exist");
    let row = run_id(&root, "css/none/missing.html");
    assert_eq!(row.status, Status::Error);
}

#[test]
fn a_missing_reftest_in_a_relative_wpt_root_reports_the_canonical_path() {
    let temp = tempfile::tempdir_in(".").expect("temporary directory");
    let temp_name = temp.path().file_name().expect("temporary directory name");
    let root = PathBuf::from(temp_name).join("wpt");
    std::fs::create_dir(&root).expect("WPT root");
    let canonical_missing = std::fs::canonicalize(&root)
        .expect("canonical WPT root")
        .join("missing.html");

    let row = run_id(&root, "missing.html");

    assert_eq!(row.status, Status::Error);
    assert!(
        row.detail.starts_with(&format!(
            "discover: I/O error at {}:",
            canonical_missing.display()
        )),
        "{}",
        row.detail
    );
}

#[test]
fn a_parent_traversal_id_is_rejected_before_reading_outside_the_wpt_root() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("wpt");
    std::fs::create_dir(&root).expect("WPT root");
    std::fs::write(temp.path().join("outside.html"), "<html></html>").expect("outside file");

    let row = run_id(&root, "../outside.html");

    assert_eq!(row.status, Status::Error);
    assert!(row.detail.contains("invalid WPT test id"), "{}", row.detail);
}

#[test]
fn a_parsing_id_with_parent_traversal_is_rejected_before_dispatch() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("wpt");
    std::fs::create_dir(&root).expect("WPT root");
    std::fs::write(temp.path().join("outside.html"), "<html></html>").expect("outside file");

    let row = run_id(&root, "css/parsing/../../../outside.html");

    assert_eq!(row.status, Status::Error);
    assert!(row.detail.contains("invalid WPT test id"), "{}", row.detail);
}

#[test]
fn an_absolute_id_is_rejected_before_reading_outside_the_wpt_root() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("wpt");
    std::fs::create_dir(&root).expect("WPT root");
    let outside = temp.path().join("outside.html");
    std::fs::write(&outside, "<html></html>").expect("outside file");

    let row = run_id(&root, outside.to_str().expect("UTF-8 temporary path"));

    assert_eq!(row.status, Status::Error);
    assert!(row.detail.contains("invalid WPT test id"), "{}", row.detail);
}

#[cfg(unix)]
#[test]
fn a_symlink_to_outside_the_wpt_root_is_rejected() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("wpt");
    std::fs::create_dir(&root).expect("WPT root");
    let outside = temp.path().join("outside.html");
    std::fs::write(&outside, "<html></html>").expect("outside file");
    symlink(&outside, root.join("link.html")).expect("outside symlink");

    let row = run_id(&root, "link.html");

    assert_eq!(row.status, Status::Error);
    assert!(
        row.detail.contains("outside the WPT root"),
        "{}",
        row.detail
    );
}

#[test]
fn a_valid_nested_relative_id_reaches_the_reftest_runner() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("wpt");
    let nested = root.join("nested/subdir");
    std::fs::create_dir_all(&nested).expect("nested WPT directory");
    std::fs::write(nested.join("page.html"), "<html></html>").expect("test file");

    let row = run_id(&root, "nested/subdir/page.html");

    assert_eq!(row.status, Status::Error);
    assert!(row.detail.contains("no reftest"), "{}", row.detail);

    let variant = run_id(&root, "nested/subdir/page.html?variant=dark");

    assert_eq!(variant.status, Status::Error);
    assert!(variant.detail.contains("no reftest"), "{}", variant.detail);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn a_relative_wpt_root_still_runs_dynamic_reftests() {
    // Cargo runs unit tests with the crate directory as the working directory,
    // so this root is relative.
    let root = PathBuf::from("../../target/wpt");
    let row = run_id(
        &root,
        "css/css-text/text-autospace/text-autospace-vertical-upright-001.html",
    );
    assert_eq!(row.status, Status::Pass, "{}", row.detail);
    // A dynamic reftest used to fail up front with "needs an absolute path".
    // Whatever it reports now, it must be the test's own outcome.
    let dynamic = run_id(
        &root,
        "css/css-backgrounds/background-attachment-fixed-block-002.html",
    );
    assert!(
        !dynamic.detail.contains("absolute path"),
        "{}",
        dynamic.detail
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn a_reftest_that_only_passes_without_local_images_still_reports_pass() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let row = run_id(
        &root,
        "css/css-backgrounds/background-position-three-four-values.html",
    );
    assert_eq!(row.status, Status::Pass, "{}", row.detail);
}

fn noted(id: &str) -> Row {
    Row {
        id: id.to_owned(),
        status: Status::Pass,
        detail: FALLBACK_NOTE.to_owned(),
    }
}

#[test]
fn a_pass_that_needed_the_resource_free_fallback_is_marked_weakened_by_diff() {
    let before = vec![row("was-clean", Status::Pass), noted("was-noted")];
    let after = vec![noted("was-clean"), noted("was-noted")];
    let result = diff(&before, &after);
    assert_eq!(result.weakened, vec!["was-clean"]);
    assert!(result.regressions.is_empty());
}

#[test]
fn a_fallback_pass_that_becomes_clean_is_not_weakened() {
    let before = vec![noted("a")];
    let after = vec![row("a", Status::Pass)];
    assert!(diff(&before, &after).weakened.is_empty());
}

#[test]
fn the_fallback_note_survives_a_tsv_round_trip() {
    let rows = vec![noted("a")];
    assert_eq!(parse_tsv(&write_tsv(&rows)).expect("parse"), rows);
}

#[test]
fn combine_keeps_the_note_only_when_the_deciding_pass_needed_the_fallback() {
    let only_fallback = vec![pair(ReftestKind::Match, Status::Pass, FALLBACK_NOTE)];
    assert_eq!(
        combine_pairs(only_fallback),
        (Status::Pass, FALLBACK_NOTE.to_owned())
    );
    let clean_alternative = vec![
        pair(ReftestKind::Match, Status::Pass, FALLBACK_NOTE),
        pair(ReftestKind::Match, Status::Pass, ""),
    ];
    assert_eq!(
        combine_pairs(clean_alternative),
        (Status::Pass, String::new())
    );
    let noted_mismatch = vec![
        pair(ReftestKind::Match, Status::Pass, ""),
        pair(ReftestKind::Mismatch, Status::Pass, FALLBACK_NOTE),
    ];
    assert_eq!(
        combine_pairs(noted_mismatch),
        (Status::Pass, FALLBACK_NOTE.to_owned())
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn a_reftest_that_only_passes_without_local_images_carries_the_note() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let row = run_id(
        &root,
        "css/css-backgrounds/background-position-three-four-values.html",
    );
    assert_eq!(row.status, Status::Pass, "{}", row.detail);
    assert_eq!(row.detail, FALLBACK_NOTE);
}

#[test]
fn the_baseline_runner_rejects_the_removed_ifc_flags() {
    for flag in ["--ifc", "--no-ifc"] {
        let error = parse_report_args(&[flag.to_owned()]).expect_err(flag);
        assert!(error.contains(flag), "{flag}: {error}");
    }
}

#[test]
fn the_wpt_fonts_flag_still_parses_and_changes_nothing() {
    // The WPT fonts are always used; the flag names that default.
    let default = parse_report_args(&[]).expect("args");
    let options = parse_report_args(&["--wpt-fonts".to_owned()]).expect("args");
    assert_eq!(options, default);
}
