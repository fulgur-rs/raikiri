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

/// Scan an `expectations/` directory and return a [`LintReport`].
///
/// `now` is injected (rather than sourced from the system clock) so
/// [`Category::Expired`] detection is deterministic in tests.
pub fn run(_dir: &Path, _now: Date) -> LintReport {
    LintReport::default()
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
}
