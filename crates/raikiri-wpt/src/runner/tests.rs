use super::*;
use crate::expectations::{
    Baseline, Deprecated, ExpectedFailure, ExpectedFailures, KnownIssues, Quarantine, TrackedWpt,
};
use crate::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root};

fn empty_set() -> ExpectationSet {
    ExpectationSet {
        tracked: TrackedWpt { entries: vec![] },
        known_issues: KnownIssues { entries: vec![] },
        baseline: Baseline {
            entries: std::collections::HashSet::new(),
        },
        expected_failures: ExpectedFailures { entries: vec![] },
        quarantine: Quarantine { entries: vec![] },
        deprecated: Deprecated {
            entries: std::collections::HashSet::new(),
        },
    }
}

#[test]
fn test_outcome_variants_are_constructible() {
    assert!(matches!(TestOutcome::Pass, TestOutcome::Pass));
    assert!(matches!(
        TestOutcome::Fail("boom".to_owned()),
        TestOutcome::Fail(_)
    ));
    assert!(matches!(
        TestOutcome::Skip("non-goal".to_owned()),
        TestOutcome::Skip(_)
    ));
    assert!(matches!(TestOutcome::Quarantined, TestOutcome::Quarantined));
}

#[test]
fn classify_deprecated_takes_precedence() {
    let mut set = empty_set();
    set.deprecated.entries.insert("css/foo.html".to_owned());
    let runner = WptRunner::new(set);
    assert!(matches!(
        runner.classify("css/foo.html"),
        Some(TestOutcome::Skip(_))
    ));
}

#[test]
fn classify_unknown_returns_none() {
    let runner = WptRunner::new(empty_set());
    assert!(runner.classify("css/bar.html").is_none());
}

#[test]
fn run_test_filtered_returns_skip() {
    let mut set = empty_set();
    set.deprecated.entries.insert("a.html".to_owned());
    let runner = WptRunner::new(set);
    let exec = runner.run_test("a.html");
    assert!(matches!(exec.outcome, TestOutcome::Skip(_)));
}

#[test]
fn tolerance_constants_distinct() {
    assert_ne!(Tolerance::EXACT, Tolerance::TIER2);
    assert_ne!(Tolerance::TIER2, Tolerance::TIER3);
}

#[test]
fn classify_quarantined_entry_returns_quarantined() {
    use crate::expectations::{
        ArchFilter, PlatformFilter, QuarantineEntry, RendererFilter, ToleranceFilter,
    };
    let mut set = empty_set();
    set.quarantine.entries.push(QuarantineEntry {
        test_id: "css/flaky.html".to_owned(),
        platform: PlatformFilter::Any,
        arch: ArchFilter::Any,
        renderer: RendererFilter::Any,
        tolerance: ToleranceFilter::Any,
        reason: "flaky".to_owned(),
        issue_link: "https://example.com/issue/1".to_owned(),
        added_date: time::Date::from_calendar_date(2026, time::Month::January, 1).unwrap(),
        line_no: 1,
    });
    let runner = WptRunner::new(set);
    assert!(matches!(
        runner.classify("css/flaky.html"),
        Some(TestOutcome::Quarantined)
    ));
}

#[test]
fn classify_known_issue_pattern_returns_skip() {
    let mut set = empty_set();
    set.known_issues
        .entries
        .push(("css/contain/".to_owned(), "non-goal".to_owned()));
    set.known_issues
        .entries
        .push(("css/exact.html".to_owned(), "non-goal".to_owned()));
    let runner = WptRunner::new(set);
    assert!(matches!(
        runner.classify("css/contain/foo.html"),
        Some(TestOutcome::Skip(_))
    ));
    assert!(matches!(
        runner.classify("css/exact.html"),
        Some(TestOutcome::Skip(_))
    ));
}

#[test]
fn exact_expected_failure_overrides_only_a_matching_known_issue_prefix() {
    let id = "css/example/test.html?mode=ja";
    let mut set = empty_set();
    set.known_issues
        .entries
        .push(("css/example/".to_owned(), "non-goal".to_owned()));
    set.expected_failures.entries.push(ExpectedFailure {
        test_id: id.to_owned(),
        reason: "Japanese variant differs".to_owned(),
        issue_id: "raikiri-spike-6qrnr.15.15".to_owned(),
        added_date: time::macros::date!(2026 - 10 - 04),
        review_by: time::macros::date!(2026 - 11 - 04),
        line_no: 1,
    });
    let runner = WptRunner::new(set);

    assert!(runner.classify(id).is_none());
    assert!(matches!(
        runner.classify("css/example/test.html"),
        Some(TestOutcome::Skip(_))
    ));
}

#[test]
fn run_test_requires_a_discovered_pair_for_an_expected_failure() {
    let id = "css/example/test.html?mode=ja";
    let mut set = empty_set();
    set.expected_failures.entries.push(ExpectedFailure {
        test_id: id.to_owned(),
        reason: "Japanese variant differs".to_owned(),
        issue_id: "raikiri-spike-6qrnr.15.15".to_owned(),
        added_date: time::macros::date!(2026 - 10 - 04),
        review_by: time::macros::date!(2026 - 11 - 04),
        line_no: 1,
    });
    let execution = WptRunner::new(set).run_test(id);

    assert!(
        matches!(execution.outcome, TestOutcome::Error(_)),
        "a filtered skip would hide that the expected failure was not executed"
    );
}

#[test]
fn run_test_unfiltered_returns_no_pair_skip() {
    let runner = WptRunner::new(empty_set());
    let exec = runner.run_test("css/bar.html");
    assert_eq!(exec.test_id, "css/bar.html");
    assert!(matches!(exec.outcome, TestOutcome::Skip(_)));
}

#[test]
fn expectations_accessor_returns_loaded_set() {
    let runner = WptRunner::new(empty_set());
    assert!(runner.expectations().deprecated.entries.is_empty());
}

#[test]
fn exact_query_expected_failure_reclassifies_render_results_but_not_errors() {
    let directory = tempfile::tempdir().unwrap();
    let test = directory.path().join("test.html");
    let reference = directory.path().join("reference.html");
    let id = "css/example/test.html?mode=red";
    let html = "<!DOCTYPE html><html><head><link rel='match' href='reference.html'></head><body style='margin:0'><script>document.body.style.background = location.search === '?mode=red' ? 'red' : 'green';</script></body></html>";
    let green = "<!DOCTYPE html><html><body style='margin:0;background:green'></body></html>";
    std::fs::write(&test, html).unwrap();
    std::fs::write(&reference, green).unwrap();

    let mut pairs = discover_pairs_for_file_with_wpt_root(&test, Some(directory.path()))
        .unwrap()
        .into_iter();
    let pair = pairs.next().expect("match relation should produce a pair");
    assert!(pairs.next().is_none());
    let mut set = empty_set();
    set.expected_failures.entries.push(ExpectedFailure {
        test_id: id.to_owned(),
        reason: "query variant paints the wrong color".to_owned(),
        issue_id: "raikiri-spike-6qrnr.15.15".to_owned(),
        added_date: time::macros::date!(2026 - 10 - 04),
        review_by: time::macros::date!(2026 - 11 - 04),
        line_no: 1,
    });
    let runner = WptRunner::new(set);
    let config = ReftestConfig {
        width: 8,
        height: 8,
        tolerance: Tolerance::EXACT,
        ..ReftestConfig::default()
    };

    let failed = runner.run_reftest_pair_for_test_id(id, &pair, config, "?mode=red");
    assert!(
        matches!(failed.outcome, TestOutcome::XFail { .. }),
        "expected the pixel mismatch to be XFAIL, got {:?}",
        failed.outcome
    );

    std::fs::write(&test, green).unwrap();
    let passed = runner.run_reftest_pair_for_test_id(id, &pair, config, "?mode=red");
    assert!(
        matches!(passed.outcome, TestOutcome::XPass { .. }),
        "a fixed test must become XPASS, got {:?}",
        passed.outcome
    );

    std::fs::remove_file(&test).unwrap();
    let errored = runner.run_reftest_pair_for_test_id(id, &pair, config, "?mode=red");
    assert!(
        matches!(errored.outcome, TestOutcome::Error(_)),
        "a render error must not become XFAIL/XPASS: {:?}",
        errored.outcome
    );
}

#[test]
fn discovery_uses_root_relative_ids_for_expected_failure_matching() {
    let directory = tempfile::tempdir().unwrap();
    let test_dir = directory.path().join("css/example");
    std::fs::create_dir_all(&test_dir).unwrap();
    std::fs::write(
        test_dir.join("test.html"),
        "<!DOCTYPE html><html><head><link rel='match' href='reference.html'></head><body style='margin:0;background:red'></body></html>",
    )
    .unwrap();
    std::fs::write(
        test_dir.join("reference.html"),
        "<!DOCTYPE html><html><body style='margin:0;background:green'></body></html>",
    )
    .unwrap();

    let mut set = empty_set();
    set.expected_failures.entries.push(ExpectedFailure {
        test_id: "css/example/test.html".to_owned(),
        reason: "background color differs".to_owned(),
        issue_id: "raikiri-spike-6qrnr.15.15".to_owned(),
        added_date: time::macros::date!(2026 - 10 - 04),
        review_by: time::macros::date!(2026 - 11 - 04),
        line_no: 1,
    });

    let results = WptRunner::new(set).run_all_under(directory.path(), ReftestConfig::default());

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.pair_test_id, "css/example/test.html");
    assert!(
        matches!(results[0].0.outcome, TestOutcome::XFail { .. }),
        "discovery should classify using root-relative test ids: {:?}",
        results[0].0.outcome
    );
}
