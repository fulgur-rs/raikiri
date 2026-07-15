//! Lint checks for `expectations/` files (spec §12.10).
//!
//! Detects 4 categories of issues:
//! - [`Category::Malformed`] — shape/enum/encoding errors (via
//!   [`crate::expectations::ExpectError`] surfaced through this module)
//! - [`Category::Duplicate`] — repeat rows within a single file
//! - [`Category::Conflicting`] — same `test_id` in multiple expectations files
//! - [`Category::Expired`] — quarantine entries older than 90 days
//!
//! Consumed by the `validate-expectations` bin. Malformed / Duplicate /
//! Conflicting cause CI to fail; Expired is warning-only.

use std::path::Path;

use time::Date;

use crate::expectations::{Baseline, Deprecated, ExpectError, KnownIssues, Quarantine, TrackedWpt};

/// Aggregate lint result for a single `expectations/` directory scan.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct LintReport {
    /// All issues detected across the 4 categories.
    pub issues: Vec<LintIssue>,
}

impl LintReport {
    /// True if any issue would block CI (all categories except [`Category::Expired`]).
    pub fn has_failures(&self) -> bool {
        self.issues.iter().any(|i| i.category.is_failure())
    }

    /// True if no issues were detected.
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }
}

/// A single lint finding.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LintIssue {
    /// Which of the 4 categories this issue belongs to.
    pub category: Category,
    /// File the issue anchors to (logical name or full path).
    pub file: String,
    /// 1-based line number, when the issue anchors to a specific line.
    pub line_no: Option<usize>,
    /// Human-readable description of the issue.
    pub message: String,
}

/// The 4 spec §12.10 detection categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Category {
    /// Wrong shape / unknown enum / bad encoding.
    Malformed,
    /// Same row appears twice in a single file.
    Duplicate,
    /// Same test id appears in multiple expectations files.
    Conflicting,
    /// Quarantine entry older than 90 days (warning only).
    Expired,
}

impl Category {
    /// True for all categories except [`Category::Expired`].
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::Expired)
    }
}

/// Successfully-loaded content per file, alongside raw text used later for
/// duplicate re-scan. Fields are `None` when the file failed to parse (in
/// which case a [`Category::Malformed`] issue was recorded in `issues`).
#[derive(Default)]
struct Loaded {
    tracked: Option<TrackedWpt>,
    known_issues: Option<KnownIssues>,
    baseline: Option<Baseline>,
    quarantine: Option<Quarantine>,
    deprecated: Option<Deprecated>,
    baseline_raw: Option<String>,
    quarantine_raw: Option<String>,
    deprecated_raw: Option<String>,
    issues: Vec<LintIssue>,
}

fn expect_error_to_issue(err: ExpectError, path_display: String) -> LintIssue {
    match err {
        ExpectError::Io(e) => LintIssue {
            category: Category::Malformed,
            file: path_display,
            line_no: None,
            message: format!("I/O error: {e}"),
        },
        ExpectError::MalformedLine { file, line_no, reason } => LintIssue {
            category: Category::Malformed,
            file,
            line_no: Some(line_no),
            message: format!("malformed line ({reason})"),
        },
        ExpectError::UnknownEnum { file, line_no, field, value } => LintIssue {
            category: Category::Malformed,
            file,
            line_no: Some(line_no),
            message: format!("unknown {field} value {value:?}"),
        },
    }
}

fn load_all(dir: &Path) -> Loaded {
    let mut out = Loaded::default();

    let tracked_path = dir.join("tracked-wpt.txt");
    match TrackedWpt::load(&tracked_path) {
        Ok(v) => out.tracked = Some(v),
        Err(e) => out.issues.push(expect_error_to_issue(e, tracked_path.display().to_string())),
    }

    let known_path = dir.join("known-issues.txt");
    match KnownIssues::load(&known_path) {
        Ok(v) => out.known_issues = Some(v),
        Err(e) => out.issues.push(expect_error_to_issue(e, known_path.display().to_string())),
    }

    let baseline_path = dir.join("raikiri-baseline.txt");
    match std::fs::read_to_string(&baseline_path) {
        Ok(raw) => {
            match Baseline::parse(&raw, &baseline_path.display().to_string()) {
                Ok(v) => out.baseline = Some(v),
                Err(e) => out.issues.push(expect_error_to_issue(e, baseline_path.display().to_string())),
            }
            out.baseline_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(ExpectError::Io(e), baseline_path.display().to_string())),
    }

    let quarantine_path = dir.join("quarantine.txt");
    match std::fs::read_to_string(&quarantine_path) {
        Ok(raw) => {
            match Quarantine::parse(&raw, &quarantine_path.display().to_string()) {
                Ok(v) => out.quarantine = Some(v),
                Err(e) => out.issues.push(expect_error_to_issue(e, quarantine_path.display().to_string())),
            }
            out.quarantine_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(ExpectError::Io(e), quarantine_path.display().to_string())),
    }

    let deprecated_path = dir.join("deprecated.txt");
    match std::fs::read_to_string(&deprecated_path) {
        Ok(raw) => {
            match Deprecated::parse(&raw, &deprecated_path.display().to_string()) {
                Ok(v) => out.deprecated = Some(v),
                Err(e) => out.issues.push(expect_error_to_issue(e, deprecated_path.display().to_string())),
            }
            out.deprecated_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(ExpectError::Io(e), deprecated_path.display().to_string())),
    }

    out
}

/// Scan an `expectations/` directory and return a [`LintReport`].
///
/// `now` is injected (rather than sourced from the system clock) so
/// [`Category::Expired`] detection is deterministic in tests.
pub fn run(dir: &Path, _now: Date) -> LintReport {
    let loaded = load_all(dir);
    LintReport { issues: loaded.issues }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn header_only_dir() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        write(dir.path(), "tracked-wpt.txt", "# header\n");
        write(dir.path(), "known-issues.txt", "# header\n");
        write(dir.path(), "raikiri-baseline.txt", "# header\n");
        write(dir.path(), "quarantine.txt", "# header\n");
        write(dir.path(), "deprecated.txt", "# header\n");
        dir
    }

    #[test]
    fn empty_expectations_dir_yields_empty_report() {
        let dir = header_only_dir();
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(report.is_empty(), "expected empty, got {:?}", report.issues);
        assert!(!report.has_failures());
    }

    #[test]
    fn category_is_failure_maps_expired_to_false() {
        assert!(Category::Malformed.is_failure());
        assert!(Category::Duplicate.is_failure());
        assert!(Category::Conflicting.is_failure());
        assert!(!Category::Expired.is_failure());
    }

    #[test]
    fn malformed_quarantine_row_count_becomes_lint_issue() {
        let dir = header_only_dir();
        // 3 columns instead of 8
        write(dir.path(), "quarantine.txt", "css/foo | linux | x86_64\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(report.has_failures());
        let issue = report
            .issues
            .iter()
            .find(|i| i.category == Category::Malformed)
            .expect("malformed issue expected");
        assert!(issue.file.ends_with("quarantine.txt"));
        assert_eq!(issue.line_no, Some(1));
        assert!(issue.message.contains("expected 8"), "got: {}", issue.message);
    }

    #[test]
    fn malformed_unknown_platform_becomes_lint_issue() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | plan9 | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let issue = report
            .issues
            .iter()
            .find(|i| i.category == Category::Malformed)
            .expect("malformed issue expected");
        assert!(issue.message.contains("platform"), "got: {}", issue.message);
        assert!(issue.message.contains("plan9"), "got: {}", issue.message);
    }

    #[test]
    fn malformed_known_issues_missing_reason_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "known-issues.txt", "css/foo | \n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(report.has_failures());
        assert_eq!(
            report
                .issues
                .iter()
                .filter(|i| i.category == Category::Malformed)
                .count(),
            1
        );
    }
}
