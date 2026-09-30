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
    for status in [Status::Pass, Status::Fail, Status::Error, Status::Skip] {
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
fn report_args_reject_bad_input() {
    assert!(parse_report_args(&strings(&["--nope"])).is_err());
    assert!(parse_report_args(&strings(&["--jobs"])).is_err());
    assert!(parse_report_args(&strings(&["--jobs", "0"])).is_err());
    assert!(parse_report_args(&strings(&["--limit", "many"])).is_err());
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
