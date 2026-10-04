//! Integration coverage for exact expected-failure lint rules.

use raikiri_wpt::lint::{Category, run};
use std::path::Path;
use tempfile::TempDir;

fn expectations_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "tracked-wpt.txt",
        "known-issues.txt",
        "expected-failures.txt",
        "raikiri-baseline.txt",
        "quarantine.txt",
        "deprecated.txt",
    ] {
        std::fs::write(dir.path().join(name), "# header\n").unwrap();
    }
    dir
}

#[test]
fn lint_reports_missing_expectation_files_instead_of_ignoring_them() {
    let dir = tempfile::tempdir().unwrap();

    let report = run(dir.path(), time::macros::date!(2026 - 10 - 04));

    assert!(report.has_failures());
    assert!(
        report.issues.iter().any(|issue| {
            issue.category == Category::Malformed && issue.file.ends_with("expected-failures.txt")
        }),
        "missing expected-failures.txt must be reported: {:?}",
        report.issues
    );
}

fn write(dir: &Path, file: &str, body: &str) {
    std::fs::write(dir.join(file), body).unwrap();
}

#[test]
fn expected_failure_can_override_a_broad_known_issue_prefix() {
    let dir = expectations_dir();
    write(
        dir.as_ref(),
        "known-issues.txt",
        "css/css-text/ | out of scope\n",
    );
    write(
        dir.as_ref(),
        "expected-failures.txt",
        "css/css-text/spacing.html?lang=ja | wrong spacing | raikiri-spike-6qrnr.15.15 | 2026-10-04 | 2026-11-04\n",
    );

    let report = run(dir.path(), time::macros::date!(2026 - 10 - 04));

    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.category == Category::Conflicting),
        "directory-wide known issue prefixes are intentionally overridable: {:?}",
        report.issues
    );
}

#[test]
fn expected_failure_conflicts_with_every_exact_exclusion_or_pass_pin() {
    let dir = expectations_dir();
    let id = "css/example/test.html?variant=ja";
    let expected =
        format!("{id} | text differs | raikiri-spike-6qrnr.15.15 | 2026-10-04 | 2026-11-04\n");
    write(dir.as_ref(), "expected-failures.txt", &expected);
    write(dir.as_ref(), "raikiri-baseline.txt", &format!("{id}\n"));
    write(dir.as_ref(), "deprecated.txt", &format!("{id}\n"));
    write(
        dir.as_ref(),
        "known-issues.txt",
        &format!("{id} | exact skip\n"),
    );
    write(
        dir.as_ref(),
        "quarantine.txt",
        &format!("{id} | * | * | * | * | flaky | local | 2026-10-04\n"),
    );

    let report = run(dir.path(), time::macros::date!(2026 - 10 - 04));
    let expected_conflicts: Vec<_> = report
        .issues
        .iter()
        .filter(|issue| {
            issue.category == Category::Conflicting && issue.file.ends_with("expected-failures.txt")
        })
        .collect();

    assert_eq!(expected_conflicts.len(), 4, "{:?}", report.issues);
    for other in [
        "raikiri-baseline.txt",
        "deprecated.txt",
        "quarantine.txt",
        "known-issues.txt",
    ] {
        assert!(
            expected_conflicts
                .iter()
                .any(|issue| issue.message.contains(other)),
            "missing conflict with {other}: {expected_conflicts:?}"
        );
    }
}

#[test]
fn expected_failure_duplicate_is_reported_and_review_expiry_is_warning_only() {
    let dir = expectations_dir();
    let row = "css/example/test.html | text differs | raikiri-spike-6qrnr.15.15 | 2026-08-01 | 2026-09-01\n";
    write(
        dir.as_ref(),
        "quarantine.txt",
        "css/other/test.html | linux | x86_64 | vello_cpu | low | flaky | local | 2026-01-01\n",
    );
    write(
        dir.as_ref(),
        "expected-failures.txt",
        &format!("{row}{row}"),
    );

    let report = run(dir.path(), time::macros::date!(2026 - 10 - 04));
    let duplicate = report
        .issues
        .iter()
        .find(|issue| issue.category == Category::Duplicate)
        .expect("duplicate exact ids must not disappear during parsing");
    assert_eq!(duplicate.line_no, Some(2));
    assert!(duplicate.message.contains("line 1"));
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.category == Category::Expired),
        "review deadline should be visible"
    );
    assert!(report.has_failures(), "duplicate is a lint error");

    write(dir.as_ref(), "expected-failures.txt", row);
    write(dir.as_ref(), "quarantine.txt", "# header\n");
    let expired_only = run(dir.path(), time::macros::date!(2026 - 10 - 04));
    assert!(!expired_only.has_failures(), "expiry alone is a warning");
    assert!(
        expired_only
            .issues
            .iter()
            .any(|issue| issue.category == Category::Expired)
    );
}
