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

use std::collections::HashMap;
use std::path::Path;

use time::{macros::format_description, Date, Duration};

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

/// Split `content` into 1-based `(line_no, line)` pairs, skipping blank
/// lines and `#`-comment lines. Shared by [`detect_dup_by_key`] and (per
/// Task 5) other line-level re-scans.
fn data_lines(content: &str) -> impl Iterator<Item = (usize, &str)> {
    content
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
}

/// Generic line-level duplicate scan: extracts a key per data line via
/// `key_of`, and reports a [`Category::Duplicate`] issue for every line
/// whose key was already seen, referencing the first line it appeared on.
fn detect_dup_by_key<F, K>(
    raw: &str,
    file_display: &str,
    key_of: F,
    message_of: impl Fn(&K, usize) -> String,
) -> Vec<LintIssue>
where
    F: Fn(&str) -> Option<K>,
    K: std::hash::Hash + Eq,
{
    let mut seen: HashMap<K, usize> = HashMap::new();
    let mut issues = Vec::new();
    for (line_no, line) in data_lines(raw) {
        let Some(key) = key_of(line) else { continue };
        if let Some(&first) = seen.get(&key) {
            issues.push(LintIssue {
                category: Category::Duplicate,
                file: file_display.to_owned(),
                line_no: Some(line_no),
                message: message_of(&key, first),
            });
        } else {
            seen.insert(key, line_no);
        }
    }
    issues
}

/// Line-level re-scan of the raw baseline / deprecated / quarantine text for
/// duplicate rows within a single file. Needed because [`Baseline`] and
/// [`Deprecated`] parse into a `HashSet`, which silently dedupes — this
/// re-scan operates on the raw text instead so duplicates surface as lint
/// issues rather than disappearing.
fn detect_duplicates(loaded: &Loaded, dir: &Path) -> Vec<LintIssue> {
    let mut issues = Vec::new();

    if let Some(raw) = loaded.baseline_raw.as_deref() {
        let path = dir.join("raikiri-baseline.txt").display().to_string();
        issues.extend(detect_dup_by_key(
            raw,
            &path,
            |l| Some(l.to_owned()),
            |k, first| format!("test_id {k:?} already appeared on line {first}"),
        ));
    }

    if let Some(raw) = loaded.deprecated_raw.as_deref() {
        let path = dir.join("deprecated.txt").display().to_string();
        issues.extend(detect_dup_by_key(
            raw,
            &path,
            |l| Some(l.to_owned()),
            |k, first| format!("test_id {k:?} already appeared on line {first}"),
        ));
    }

    if let Some(raw) = loaded.quarantine_raw.as_deref() {
        let path = dir.join("quarantine.txt").display().to_string();
        // Key = first 5 columns (test_id, platform, arch, renderer, tolerance).
        // Silently ignore lines that don't split into >= 5 columns; malformed
        // lines are reported by the parser via detect_malformed.
        issues.extend(detect_dup_by_key(
            raw,
            &path,
            |l| {
                let cols: Vec<&str> = l.split('|').map(str::trim).collect();
                if cols.len() < 5 {
                    return None;
                }
                Some((
                    cols[0].to_owned(),
                    cols[1].to_owned(),
                    cols[2].to_owned(),
                    cols[3].to_owned(),
                    cols[4].to_owned(),
                ))
            },
            |k, first| {
                format!(
                    "(test_id={:?}, platform={:?}, arch={:?}, renderer={:?}, tolerance={:?}) already appeared on line {}",
                    k.0, k.1, k.2, k.3, k.4, first
                )
            },
        ));
    }

    issues
}

fn detect_conflicting(loaded: &Loaded, dir: &Path) -> Vec<LintIssue> {
    use std::collections::BTreeSet;

    let baseline: BTreeSet<&str> = loaded
        .baseline
        .as_ref()
        .map(|b| b.entries.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let deprecated: BTreeSet<&str> = loaded
        .deprecated
        .as_ref()
        .map(|d| d.entries.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let quarantine: BTreeSet<&str> = loaded
        .quarantine
        .as_ref()
        .map(|q| q.entries.iter().map(|e| e.test_id.as_str()).collect())
        .unwrap_or_default();

    let baseline_path = dir.join("raikiri-baseline.txt").display().to_string();
    let deprecated_path = dir.join("deprecated.txt").display().to_string();
    let _quarantine_path = dir.join("quarantine.txt").display().to_string();

    let mut issues = Vec::new();
    let mk = |file: &str, other: &str, test_id: &str| LintIssue {
        category: Category::Conflicting,
        file: file.to_owned(),
        line_no: None,
        message: format!(
            "test_id {test_id:?} also appears in {other} (spec §12.10 precedence: resolve via PR)"
        ),
    };
    for id in deprecated.intersection(&baseline) {
        issues.push(mk(&deprecated_path, "raikiri-baseline.txt", id));
    }
    for id in deprecated.intersection(&quarantine) {
        issues.push(mk(&deprecated_path, "quarantine.txt", id));
    }
    for id in baseline.intersection(&quarantine) {
        issues.push(mk(&baseline_path, "quarantine.txt", id));
    }
    issues
}

/// Scan the `quarantine.txt` entries for parse failures (→
/// [`Category::Malformed`]) and entries older than 90 days relative to
/// `now` (→ [`Category::Expired`], warning-only).
///
/// The current parser stores `added_date` as a plain `String` (see
/// [`crate::expectations::QuarantineEntry`]); this function performs the
/// `time::Date` parse itself. Follow-up bd `raikiri-spike-md0` will migrate
/// the parser to store `time::Date` directly, at which point this parse
/// step can be removed but the 90-day threshold check stays.
fn detect_expired(loaded: &Loaded, dir: &Path, now: Date) -> Vec<LintIssue> {
    let Some(q) = loaded.quarantine.as_ref() else { return Vec::new() };
    let path = dir.join("quarantine.txt").display().to_string();
    let fmt = format_description!("[year]-[month]-[day]");
    let mut issues = Vec::new();
    for (idx, entry) in q.entries.iter().enumerate() {
        // Line number: entries appear in file order, but comments/blank
        // lines shift the parser's index. Recompute by re-scanning the
        // raw content for the idx-th data line.
        let line_no = loaded
            .quarantine_raw
            .as_deref()
            .and_then(|raw| data_lines(raw).nth(idx).map(|(n, _)| n));
        match Date::parse(&entry.added_date, &fmt) {
            Err(e) => issues.push(LintIssue {
                category: Category::Malformed,
                file: path.clone(),
                line_no,
                message: format!(
                    "added_date {:?} is not YYYY-MM-DD ({e})",
                    entry.added_date
                ),
            }),
            Ok(added) => {
                if now - added > Duration::days(90) {
                    issues.push(LintIssue {
                        category: Category::Expired,
                        file: path.clone(),
                        line_no,
                        message: format!(
                            "quarantine entry added on {} is older than 90 days (test_id={:?})",
                            entry.added_date, entry.test_id
                        ),
                    });
                }
            }
        }
    }
    issues
}

/// Scan an `expectations/` directory and return a [`LintReport`].
///
/// `now` is injected (rather than sourced from the system clock) so
/// [`Category::Expired`] detection is deterministic in tests.
pub fn run(dir: &Path, now: Date) -> LintReport {
    let loaded = load_all(dir);
    let mut issues = loaded.issues.clone();
    issues.extend(detect_duplicates(&loaded, dir));
    issues.extend(detect_conflicting(&loaded, dir));
    issues.extend(detect_expired(&loaded, dir, now));
    LintReport { issues }
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

    #[test]
    fn duplicate_baseline_test_id_becomes_lint_issue() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "raikiri-baseline.txt",
            "css/foo/bar-001\ncss/foo/bar-001\ncss/foo/baz-002\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(report.has_failures());
        let dup: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Duplicate).collect();
        assert_eq!(dup.len(), 1);
        assert!(dup[0].file.ends_with("raikiri-baseline.txt"));
        assert_eq!(dup[0].line_no, Some(2));
        assert!(dup[0].message.contains("css/foo/bar-001"), "got: {}", dup[0].message);
        assert!(dup[0].message.contains("line 1"), "got: {}", dup[0].message);
    }

    #[test]
    fn duplicate_deprecated_test_id_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "deprecated.txt", "css/x\ncss/y\ncss/x\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let dup: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Duplicate).collect();
        assert_eq!(dup.len(), 1);
        assert_eq!(dup[0].line_no, Some(3));
    }

    #[test]
    fn duplicate_quarantine_tuple_becomes_lint_issue() {
        let dir = header_only_dir();
        // Same (test_id, platform, arch, renderer, tolerance) tuple twice.
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r1 | i1 | 2026-08-01\n\
             css/foo | linux | x86_64 | vello_cpu | low | r2 | i2 | 2026-08-02\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let dup: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Duplicate).collect();
        assert_eq!(dup.len(), 1);
        assert_eq!(dup[0].line_no, Some(2));
        assert!(dup[0].message.contains("line 1"));
    }

    #[test]
    fn duplicate_quarantine_different_platform_is_not_duplicate() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r1 | i1 | 2026-08-01\n\
             css/foo | macos | x86_64 | vello_cpu | low | r2 | i2 | 2026-08-02\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(
            !report.issues.iter().any(|i| i.category == Category::Duplicate),
            "unexpected: {:?}",
            report.issues
        );
    }

    #[test]
    fn conflict_deprecated_and_baseline_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/x/y-001\n");
        write(dir.path(), "deprecated.txt", "css/x/y-001\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let conflicts: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Conflicting).collect();
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].message.contains("css/x/y-001"));
        assert!(conflicts[0].message.contains("deprecated.txt") || conflicts[0].file.ends_with("deprecated.txt"));
        assert!(conflicts[0].message.contains("raikiri-baseline.txt") || conflicts[0].file.ends_with("raikiri-baseline.txt"));
    }

    #[test]
    fn conflict_deprecated_and_quarantine_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "deprecated.txt", "css/x/y-001\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/x/y-001 | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let conflicts: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Conflicting).collect();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn conflict_baseline_and_quarantine_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/x/y-001\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/x/y-001 | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let conflicts: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Conflicting).collect();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn no_conflict_when_test_id_appears_in_only_one_file() {
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/x/y-001\n");
        write(dir.path(), "deprecated.txt", "css/other\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(!report.issues.iter().any(|i| i.category == Category::Conflicting));
    }

    #[test]
    fn expired_quarantine_older_than_90_days_becomes_warning() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-01-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16); // 196 days later
        let report = run(dir.path(), now);
        let expired: Vec<_> = report.issues.iter().filter(|i| i.category == Category::Expired).collect();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].line_no, Some(1));
        assert!(expired[0].message.contains("2026-01-01"));
        assert!(expired[0].message.contains("90 days"));
        // Expired-only reports do NOT block CI.
        assert!(!report.has_failures(), "unexpected failure with only Expired: {:?}", report.issues);
    }

    #[test]
    fn quarantine_within_90_days_is_not_expired() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-06-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16); // 45 days later
        let report = run(dir.path(), now);
        assert!(!report.issues.iter().any(|i| i.category == Category::Expired));
    }

    #[test]
    fn expired_boundary_exactly_90_days_is_not_expired() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-04-17\n",
        );
        let now = time::macros::date!(2026 - 07 - 16); // exactly 90 days
        let report = run(dir.path(), now);
        assert!(!report.issues.iter().any(|i| i.category == Category::Expired));
    }

    #[test]
    fn malformed_added_date_becomes_malformed_lint_issue() {
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r | i | not-a-date\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let malformed: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Malformed)
            .collect();
        assert_eq!(malformed.len(), 1);
        assert_eq!(malformed[0].line_no, Some(1));
        assert!(malformed[0].message.contains("added_date"));
        assert!(malformed[0].message.contains("not-a-date"));
    }
}
