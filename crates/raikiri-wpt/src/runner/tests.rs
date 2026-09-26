use super::*;
use crate::expectations::{Baseline, Deprecated, KnownIssues, Quarantine, TrackedWpt};

fn empty_set() -> ExpectationSet {
    ExpectationSet {
        tracked: TrackedWpt { entries: vec![] },
        known_issues: KnownIssues { entries: vec![] },
        baseline: Baseline {
            entries: std::collections::HashSet::new(),
        },
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
