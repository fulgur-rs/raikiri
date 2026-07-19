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

use time::{Date, Duration};

use crate::expectations::{
    ArchFilter, Baseline, Deprecated, ExpectError, KnownIssues, PlatformFilter, Quarantine,
    QuarantineEntry, RendererFilter, ToleranceFilter, TrackedWpt,
};

/// A single concrete execution environment: one row of the WPT CI matrix
/// (spec §12.10 [`Category::Conflicting`] filter-overlap analysis).
///
/// `Any` in any axis is treated as a wildcard on that axis when matching
/// against a [`QuarantineEntry`]'s filter — a matrix authored with
/// wildcards means "we run every choice of that axis" and matches any
/// quarantine filter for it. Real matrix rows are usually all-concrete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MatrixRow {
    /// OS platform this row runs on.
    pub platform: PlatformFilter,
    /// CPU architecture this row runs on.
    pub arch: ArchFilter,
    /// Renderer backend this row runs.
    pub renderer: RendererFilter,
    /// Pixel-diff tolerance this row applies.
    pub tolerance: ToleranceFilter,
}

impl MatrixRow {
    /// True if a [`QuarantineEntry`]'s filter applies to this row: each
    /// axis is either equal or one side is `Any` (wildcard).
    fn matches_entry(&self, entry: &QuarantineEntry) -> bool {
        axis_matches(entry.platform, self.platform, PlatformFilter::Any)
            && axis_matches(entry.arch, self.arch, ArchFilter::Any)
            && axis_matches(entry.renderer, self.renderer, RendererFilter::Any)
            && axis_matches(entry.tolerance, self.tolerance, ToleranceFilter::Any)
    }
}

fn axis_matches<T: PartialEq + Copy>(a: T, b: T, any: T) -> bool {
    a == any || b == any || a == b
}

/// Ordered set of [`MatrixRow`] entries that the WPT runner is expected to
/// execute (spec §12.10 filter-overlap discipline). Consumers thread this
/// into [`run_with_matrix`] to reduce `baseline ∩ quarantine`
/// [`Category::Conflicting`] false positives (a quarantine entry whose
/// filter doesn't cover any matrix row can co-exist with baseline).
///
/// Parseable via [`str::parse`] (see the [`std::str::FromStr`] impl) using
/// the semicolon/comma text format expected by the `RAIKIRI_WPT_MATRIX`
/// env var contract in `validate-expectations`.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RunMatrix {
    /// Concrete matrix rows.
    pub rows: Vec<MatrixRow>,
}

/// Parse errors for the [`std::str::FromStr`] impl of [`RunMatrix`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunMatrixParseError {
    /// A row did not have exactly 4 comma-delimited fields.
    WrongColumnCount {
        /// 1-based row index in the input string.
        row: usize,
        /// How many fields the row actually had.
        got: usize,
    },
    /// A field could not be parsed as the expected enum value.
    UnknownEnumValue {
        /// 1-based row index in the input string.
        row: usize,
        /// Axis name: `"platform"`, `"arch"`, `"renderer"`, `"tolerance"`.
        field: &'static str,
        /// The raw value that failed to parse.
        value: String,
    },
}

impl std::fmt::Display for RunMatrixParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongColumnCount { row, got } => write!(
                f,
                "run matrix row {row}: expected 4 comma-delimited fields, got {got}"
            ),
            Self::UnknownEnumValue { row, field, value } => {
                write!(f, "run matrix row {row}: unknown {field} value {value:?}")
            }
        }
    }
}

impl std::error::Error for RunMatrixParseError {}

impl RunMatrix {
    /// True if no rows are defined (matches nothing).
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// True if any row in the matrix would apply to `entry`.
    fn any_row_matches(&self, entry: &QuarantineEntry) -> bool {
        self.rows.iter().any(|r| r.matches_entry(entry))
    }
}

impl std::str::FromStr for RunMatrix {
    type Err = RunMatrixParseError;

    /// Parse `s` as `platform,arch,renderer,tolerance;...` (semicolon-
    /// delimited rows, comma-delimited fields). Whitespace around fields
    /// and rows is ignored; empty rows are skipped so trailing `;` is OK.
    ///
    /// The 4 fields accept the same enum values as `quarantine.txt`
    /// columns 2-5 (`linux`, `x86_64`, `vello_cpu`, `pixel-exact`, `*`,
    /// etc.). An empty string produces an empty [`RunMatrix`], which
    /// matches nothing.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut rows = Vec::new();
        for (i, raw_row) in s.split(';').enumerate() {
            let row = raw_row.trim();
            if row.is_empty() {
                continue;
            }
            let cols: Vec<&str> = row.split(',').map(str::trim).collect();
            if cols.len() != 4 {
                return Err(RunMatrixParseError::WrongColumnCount {
                    row: i + 1,
                    got: cols.len(),
                });
            }
            let platform = PlatformFilter::parse_str(cols[0]).ok_or_else(|| {
                RunMatrixParseError::UnknownEnumValue {
                    row: i + 1,
                    field: "platform",
                    value: cols[0].to_owned(),
                }
            })?;
            let arch = ArchFilter::parse_str(cols[1]).ok_or_else(|| {
                RunMatrixParseError::UnknownEnumValue {
                    row: i + 1,
                    field: "arch",
                    value: cols[1].to_owned(),
                }
            })?;
            let renderer = RendererFilter::parse_str(cols[2]).ok_or_else(|| {
                RunMatrixParseError::UnknownEnumValue {
                    row: i + 1,
                    field: "renderer",
                    value: cols[2].to_owned(),
                }
            })?;
            let tolerance = ToleranceFilter::parse_str(cols[3]).ok_or_else(|| {
                RunMatrixParseError::UnknownEnumValue {
                    row: i + 1,
                    field: "tolerance",
                    value: cols[3].to_owned(),
                }
            })?;
            rows.push(MatrixRow {
                platform,
                arch,
                renderer,
                tolerance,
            });
        }
        Ok(Self { rows })
    }
}

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

    /// Render as human-readable text grouped by [`Category`].
    ///
    /// Categories always emit in the order Malformed → Duplicate →
    /// Conflicting → Expired for stable CI output. Each issue is one line
    /// of the form `<file>[:<line>]: <message>`. A summary line closes.
    pub fn format_human(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for cat in [
            Category::Malformed,
            Category::Duplicate,
            Category::Conflicting,
            Category::Expired,
        ] {
            let group: Vec<&LintIssue> = self.issues.iter().filter(|i| i.category == cat).collect();
            if group.is_empty() {
                continue;
            }
            writeln!(out, "== {:?} ({}) ==", cat, group.len()).unwrap();
            for i in group {
                match i.line_no {
                    Some(n) => writeln!(out, "  {}:{}: {}", i.file, n, i.message).unwrap(),
                    None => writeln!(out, "  {}: {}", i.file, i.message).unwrap(),
                }
            }
        }
        if self.is_empty() {
            out.push_str("expectations: clean\n");
        } else {
            let n_fail = self
                .issues
                .iter()
                .filter(|i| i.category.is_failure())
                .count();
            let n_warn = self
                .issues
                .iter()
                .filter(|i| !i.category.is_failure())
                .count();
            writeln!(
                out,
                "expectations: {} failure(s), {} warning(s)",
                n_fail, n_warn
            )
            .unwrap();
        }
        out
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
    // Kept for parse-error surfacing and future cross-file checks; not read yet.
    tracked: Option<TrackedWpt>,
    // Kept for parse-error surfacing and future cross-file checks; not read yet.
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
        ExpectError::MalformedLine {
            file,
            line_no,
            reason,
        } => LintIssue {
            category: Category::Malformed,
            file,
            line_no: Some(line_no),
            message: format!("malformed line ({reason})"),
        },
        ExpectError::UnknownEnum {
            file,
            line_no,
            field,
            value,
        } => LintIssue {
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
        Err(e) => out
            .issues
            .push(expect_error_to_issue(e, tracked_path.display().to_string())),
    }

    let known_path = dir.join("known-issues.txt");
    match KnownIssues::load(&known_path) {
        Ok(v) => out.known_issues = Some(v),
        Err(e) => out
            .issues
            .push(expect_error_to_issue(e, known_path.display().to_string())),
    }

    let baseline_path = dir.join("raikiri-baseline.txt");
    match std::fs::read_to_string(&baseline_path) {
        Ok(raw) => {
            match Baseline::parse(&raw, &baseline_path.display().to_string()) {
                Ok(v) => out.baseline = Some(v),
                Err(e) => out.issues.push(expect_error_to_issue(
                    e,
                    baseline_path.display().to_string(),
                )),
            }
            out.baseline_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(
            ExpectError::Io(e),
            baseline_path.display().to_string(),
        )),
    }

    let quarantine_path = dir.join("quarantine.txt");
    match std::fs::read_to_string(&quarantine_path) {
        Ok(raw) => {
            let (v, errors) = Quarantine::parse(&raw, &quarantine_path.display().to_string());
            for e in errors {
                out.issues.push(expect_error_to_issue(
                    e,
                    quarantine_path.display().to_string(),
                ));
            }
            out.quarantine = Some(v);
            out.quarantine_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(
            ExpectError::Io(e),
            quarantine_path.display().to_string(),
        )),
    }

    let deprecated_path = dir.join("deprecated.txt");
    match std::fs::read_to_string(&deprecated_path) {
        Ok(raw) => {
            match Deprecated::parse(&raw, &deprecated_path.display().to_string()) {
                Ok(v) => out.deprecated = Some(v),
                Err(e) => out.issues.push(expect_error_to_issue(
                    e,
                    deprecated_path.display().to_string(),
                )),
            }
            out.deprecated_raw = Some(raw);
        }
        Err(e) => out.issues.push(expect_error_to_issue(
            ExpectError::Io(e),
            deprecated_path.display().to_string(),
        )),
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

/// Compute `baseline ∩ quarantine` / `deprecated ∩ baseline` /
/// `deprecated ∩ quarantine` [`Category::Conflicting`] issues.
///
/// When `matrix` is `Some`, `baseline ∩ quarantine` collisions are
/// filtered so a quarantine entry whose filter doesn't cover any
/// [`MatrixRow`] doesn't count as a conflict (per spec §12.10:
/// the quarantine "flaky on env X" doesn't conflict with the baseline
/// "passes on env Y" as long as X and Y are disjoint on any axis).
/// Deprecated ↔ baseline / deprecated ↔ quarantine remain matrix-
/// independent — deprecated is unconditional exclusion, its overlap
/// with anything is always a conflict.
fn detect_conflicting(loaded: &Loaded, dir: &Path, matrix: Option<&RunMatrix>) -> Vec<LintIssue> {
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
    let quarantine_ids: BTreeSet<&str> = loaded
        .quarantine
        .as_ref()
        .map(|q| q.entries.iter().map(|e| e.test_id.as_str()).collect())
        .unwrap_or_default();

    let baseline_path = dir.join("raikiri-baseline.txt").display().to_string();
    let deprecated_path = dir.join("deprecated.txt").display().to_string();

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
    for id in deprecated.intersection(&quarantine_ids) {
        issues.push(mk(&deprecated_path, "quarantine.txt", id));
    }
    for id in baseline.intersection(&quarantine_ids) {
        // Optional filter-overlap suppression: with a matrix supplied,
        // report a conflict only if at least one quarantine entry for
        // this test_id actually applies to a matrix row.
        if let (Some(m), Some(q)) = (matrix, loaded.quarantine.as_ref()) {
            let applies = q
                .entries
                .iter()
                .filter(|e| e.test_id == *id)
                .any(|e| m.any_row_matches(e));
            if !applies {
                continue;
            }
        }
        issues.push(mk(&baseline_path, "quarantine.txt", id));
    }
    issues
}

/// Scan the `quarantine.txt` entries for entries older than 90 days
/// relative to `now` (→ [`Category::Expired`], warning-only).
///
/// Parse failures for `added_date` are already surfaced upstream as
/// [`Category::Malformed`] via [`crate::expectations::Quarantine::parse`],
/// so this function only handles the 90-day threshold check.
fn detect_expired(loaded: &Loaded, dir: &Path, now: Date) -> Vec<LintIssue> {
    let Some(q) = loaded.quarantine.as_ref() else {
        return Vec::new();
    };
    let path = dir.join("quarantine.txt").display().to_string();
    let mut issues = Vec::new();
    for entry in q.entries.iter() {
        if now - entry.added_date > Duration::days(90) {
            issues.push(LintIssue {
                category: Category::Expired,
                file: path.clone(),
                line_no: Some(entry.line_no),
                message: format!(
                    "quarantine entry added on {} is older than 90 days (test_id={:?})",
                    entry.added_date, entry.test_id
                ),
            });
        }
    }
    issues
}

/// Scan an `expectations/` directory and return a [`LintReport`].
///
/// `now` is injected (rather than sourced from the system clock) so
/// [`Category::Expired`] detection is deterministic in tests.
///
/// Equivalent to [`run_with_matrix`] called with `matrix = None`: every
/// `baseline ∩ quarantine` overlap surfaces as [`Category::Conflicting`]
/// (safe upper bound). Use [`run_with_matrix`] when a WPT run matrix is
/// available and spurious conflicts should be filtered.
pub fn run(dir: &Path, now: Date) -> LintReport {
    run_with_matrix(dir, now, None)
}

/// Like [`run`] but takes a [`RunMatrix`] used for filter-overlap
/// suppression of `baseline ∩ quarantine` [`Category::Conflicting`]
/// issues (spec §12.10).
///
/// See [`RunMatrix`] for how to build one; `validate-expectations` reads
/// the `RAIKIRI_WPT_MATRIX` env var and threads it into this function
/// when defined.
pub fn run_with_matrix(dir: &Path, now: Date, matrix: Option<&RunMatrix>) -> LintReport {
    let loaded = load_all(dir);
    let mut issues = loaded.issues.clone();
    issues.extend(detect_duplicates(&loaded, dir));
    issues.extend(detect_conflicting(&loaded, dir, matrix));
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
        assert!(
            issue.message.contains("expected 8"),
            "got: {}",
            issue.message
        );
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
        let dup: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Duplicate)
            .collect();
        assert_eq!(dup.len(), 1);
        assert!(dup[0].file.ends_with("raikiri-baseline.txt"));
        assert_eq!(dup[0].line_no, Some(2));
        assert!(
            dup[0].message.contains("css/foo/bar-001"),
            "got: {}",
            dup[0].message
        );
        assert!(dup[0].message.contains("line 1"), "got: {}", dup[0].message);
    }

    #[test]
    fn duplicate_deprecated_test_id_becomes_lint_issue() {
        let dir = header_only_dir();
        write(dir.path(), "deprecated.txt", "css/x\ncss/y\ncss/x\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let dup: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Duplicate)
            .collect();
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
        let dup: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Duplicate)
            .collect();
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
            !report
                .issues
                .iter()
                .any(|i| i.category == Category::Duplicate),
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
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].message.contains("css/x/y-001"));
        assert!(
            conflicts[0].message.contains("deprecated.txt")
                || conflicts[0].file.ends_with("deprecated.txt")
        );
        assert!(
            conflicts[0].message.contains("raikiri-baseline.txt")
                || conflicts[0].file.ends_with("raikiri-baseline.txt")
        );
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
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
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
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn no_conflict_when_test_id_appears_in_only_one_file() {
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/x/y-001\n");
        write(dir.path(), "deprecated.txt", "css/other\n");
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.category == Category::Conflicting)
        );
    }

    #[test]
    fn conflict_test_id_in_all_three_files_emits_three_pairwise_issues() {
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/x/y-001\n");
        write(dir.path(), "deprecated.txt", "css/x/y-001\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/x/y-001 | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
        assert_eq!(
            conflicts.len(),
            3,
            "expected 3 pairwise conflicts, got {:?}",
            conflicts
        );
        // Each of the 3 pairs must be present. Check by (file, other-in-message) shape.
        assert!(
            conflicts.iter().any(|i| i.file.ends_with("deprecated.txt")
                && i.message.contains("raikiri-baseline.txt")),
            "missing deprecated∩baseline pair: {conflicts:?}"
        );
        assert!(
            conflicts.iter().any(|i| i.file.ends_with("deprecated.txt")
                && i.message.contains("quarantine.txt")),
            "missing deprecated∩quarantine pair: {conflicts:?}"
        );
        assert!(
            conflicts
                .iter()
                .any(|i| i.file.ends_with("raikiri-baseline.txt")
                    && i.message.contains("quarantine.txt")),
            "missing baseline∩quarantine pair: {conflicts:?}"
        );
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
        let expired: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Expired)
            .collect();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].line_no, Some(1));
        assert!(expired[0].message.contains("2026-01-01"));
        assert!(expired[0].message.contains("90 days"));
        // Expired-only reports do NOT block CI.
        assert!(
            !report.has_failures(),
            "unexpected failure with only Expired: {:?}",
            report.issues
        );
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
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.category == Category::Expired)
        );
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
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.category == Category::Expired)
        );
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

    #[test]
    fn mixed_malformed_and_valid_expired_surfaces_both() {
        // A malformed row (wrong column count) plus a valid but > 90 days
        // old row must produce both a Malformed and an Expired lint issue —
        // the malformed row must not swallow the expired check for the
        // surviving valid entry.
        let dir = header_only_dir();
        write(
            dir.path(),
            "quarantine.txt",
            "\
css/bad | linux | x86_64
css/old | linux | x86_64 | vello_cpu | low | r | i | 2026-01-01
",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now);

        let malformed: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Malformed)
            .collect();
        assert_eq!(malformed.len(), 1, "got: {malformed:?}");
        assert_eq!(malformed[0].line_no, Some(1));
        assert!(
            malformed[0].message.contains("expected 8"),
            "got: {}",
            malformed[0].message
        );

        let expired: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Expired)
            .collect();
        assert_eq!(expired.len(), 1, "got: {expired:?}");
        assert!(
            expired[0].message.contains("css/old"),
            "got: {}",
            expired[0].message
        );
        assert_eq!(expired[0].line_no, Some(2));
    }

    #[test]
    fn format_human_groups_by_category_and_records_counts() {
        let report = LintReport {
            issues: vec![
                LintIssue {
                    category: Category::Malformed,
                    file: "raikiri-baseline.txt".to_owned(),
                    line_no: Some(2),
                    message: "bad".to_owned(),
                },
                LintIssue {
                    category: Category::Duplicate,
                    file: "raikiri-baseline.txt".to_owned(),
                    line_no: Some(3),
                    message: "dup".to_owned(),
                },
                LintIssue {
                    category: Category::Expired,
                    file: "quarantine.txt".to_owned(),
                    line_no: Some(1),
                    message: "old".to_owned(),
                },
            ],
        };
        let out = report.format_human();
        insta::assert_snapshot!("format_human_mixed_categories", out);
    }

    #[test]
    fn format_human_empty_report_snapshot() {
        let out = LintReport::default().format_human();
        insta::assert_snapshot!("format_human_empty", out);
    }

    // ── RunMatrix — spec §12.10 filter-overlap analysis ────────────────

    #[test]
    fn run_matrix_from_str_parses_two_rows() {
        let m = "linux,x86_64,vello_cpu,pixel-exact;macos,aarch64,skia,low"
            .parse::<RunMatrix>()
            .expect("valid matrix");
        assert_eq!(m.rows.len(), 2);
        assert_eq!(m.rows[0].platform, PlatformFilter::Linux);
        assert_eq!(m.rows[0].arch, ArchFilter::X86_64);
        assert_eq!(m.rows[0].renderer, RendererFilter::VelloCpu);
        assert_eq!(m.rows[0].tolerance, ToleranceFilter::PixelExact);
        assert_eq!(m.rows[1].platform, PlatformFilter::MacOs);
        assert_eq!(m.rows[1].renderer, RendererFilter::Skia);
    }

    #[test]
    fn run_matrix_from_str_accepts_wildcards() {
        let m = "*,x86_64,vello_cpu,*".parse::<RunMatrix>().expect("valid");
        assert_eq!(m.rows[0].platform, PlatformFilter::Any);
        assert_eq!(m.rows[0].tolerance, ToleranceFilter::Any);
    }

    #[test]
    fn run_matrix_from_str_empty_string_yields_zero_rows() {
        let m = "".parse::<RunMatrix>().expect("empty ok");
        assert!(m.is_empty());
    }

    #[test]
    fn run_matrix_from_str_trailing_semicolon_ignored() {
        let m = "linux,x86_64,vello_cpu,pixel-exact;"
            .parse::<RunMatrix>()
            .expect("ok");
        assert_eq!(m.rows.len(), 1);
    }

    #[test]
    fn run_matrix_from_str_rejects_wrong_column_count() {
        let err = "linux,x86_64,vello_cpu".parse::<RunMatrix>().unwrap_err();
        match err {
            RunMatrixParseError::WrongColumnCount { row, got } => {
                assert_eq!(row, 1);
                assert_eq!(got, 3);
            }
            other => panic!("expected WrongColumnCount, got {other:?}"),
        }
    }

    #[test]
    fn run_matrix_from_str_rejects_unknown_platform() {
        let err = "plan9,x86_64,vello_cpu,low"
            .parse::<RunMatrix>()
            .unwrap_err();
        match err {
            RunMatrixParseError::UnknownEnumValue { row, field, value } => {
                assert_eq!(row, 1);
                assert_eq!(field, "platform");
                assert_eq!(value, "plan9");
            }
            other => panic!("expected UnknownEnumValue, got {other:?}"),
        }
    }

    #[test]
    fn matrix_row_matches_entry_when_axes_are_equal() {
        let row = MatrixRow {
            platform: PlatformFilter::Linux,
            arch: ArchFilter::X86_64,
            renderer: RendererFilter::VelloCpu,
            tolerance: ToleranceFilter::Low,
        };
        let entry = QuarantineEntry {
            test_id: "css/foo".to_owned(),
            platform: PlatformFilter::Linux,
            arch: ArchFilter::X86_64,
            renderer: RendererFilter::VelloCpu,
            tolerance: ToleranceFilter::Low,
            reason: String::new(),
            issue_link: String::new(),
            added_date: time::macros::date!(2026 - 08 - 01),
            line_no: 1,
        };
        assert!(row.matches_entry(&entry));
    }

    #[test]
    fn matrix_row_matches_entry_via_wildcards() {
        let row = MatrixRow {
            platform: PlatformFilter::Linux,
            arch: ArchFilter::X86_64,
            renderer: RendererFilter::VelloCpu,
            tolerance: ToleranceFilter::Low,
        };
        let entry_wild = QuarantineEntry {
            test_id: "css/foo".to_owned(),
            platform: PlatformFilter::Any, // wildcard on entry side matches
            arch: ArchFilter::X86_64,
            renderer: RendererFilter::VelloCpu,
            tolerance: ToleranceFilter::Any,
            reason: String::new(),
            issue_link: String::new(),
            added_date: time::macros::date!(2026 - 08 - 01),
            line_no: 1,
        };
        assert!(row.matches_entry(&entry_wild));
    }

    #[test]
    fn matrix_row_does_not_match_entry_when_axis_disjoint() {
        let row = MatrixRow {
            platform: PlatformFilter::Linux,
            arch: ArchFilter::X86_64,
            renderer: RendererFilter::VelloCpu,
            tolerance: ToleranceFilter::Low,
        };
        let entry = QuarantineEntry {
            test_id: "css/foo".to_owned(),
            platform: PlatformFilter::Windows, // disjoint on platform
            arch: ArchFilter::X86_64,
            renderer: RendererFilter::VelloCpu,
            tolerance: ToleranceFilter::Low,
            reason: String::new(),
            issue_link: String::new(),
            added_date: time::macros::date!(2026 - 08 - 01),
            line_no: 1,
        };
        assert!(!row.matches_entry(&entry));
    }

    #[test]
    fn conflict_baseline_and_quarantine_suppressed_when_filter_disjoint_from_matrix() {
        // Baseline says css/foo passes.
        // Quarantine flags it as flaky only on Windows.
        // Matrix says CI runs on Linux only. → No conflict (quarantine
        // filter is disjoint from every matrix row on the platform axis).
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/foo\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | windows | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let matrix = "linux,x86_64,vello_cpu,low".parse::<RunMatrix>().unwrap();
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run_with_matrix(dir.path(), now, Some(&matrix));
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
        assert!(
            conflicts.is_empty(),
            "expected filter-overlap suppression, got {conflicts:?}"
        );
    }

    #[test]
    fn conflict_baseline_and_quarantine_reported_when_matrix_matches() {
        // Same setup, but quarantine covers Linux → matrix matches → conflict.
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/foo\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let matrix = "linux,x86_64,vello_cpu,low".parse::<RunMatrix>().unwrap();
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run_with_matrix(dir.path(), now, Some(&matrix));
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].message.contains("css/foo"));
    }

    #[test]
    fn conflict_baseline_and_quarantine_reported_when_wildcard_entry_matches_any_row() {
        // Quarantine uses `*` on platform axis → matches every matrix row.
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/foo\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | * | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let matrix = "windows,x86_64,vello_cpu,low".parse::<RunMatrix>().unwrap();
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run_with_matrix(dir.path(), now, Some(&matrix));
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn conflict_baseline_and_quarantine_matrix_covers_via_second_entry() {
        // Two quarantine entries share test_id but different filters.
        // First entry disjoint from matrix (Windows-only), second matches
        // matrix (Linux). Any-match semantics → conflict reported.
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/foo\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | windows | x86_64 | vello_cpu | low | r1 | i1 | 2026-08-01\n\
             css/foo | linux | x86_64 | vello_cpu | low | r2 | i2 | 2026-08-02\n",
        );
        let matrix = "linux,x86_64,vello_cpu,low".parse::<RunMatrix>().unwrap();
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run_with_matrix(dir.path(), now, Some(&matrix));
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn conflict_baseline_and_quarantine_no_matrix_preserves_upper_bound() {
        // Sanity: without matrix, behavior is unchanged (every overlap is
        // a conflict). This is the safe default `run()` uses.
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/foo\n");
        write(
            dir.path(),
            "quarantine.txt",
            "css/foo | windows | x86_64 | vello_cpu | low | r | i | 2026-08-01\n",
        );
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run(dir.path(), now); // no matrix
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn conflict_deprecated_still_reported_regardless_of_matrix() {
        // Deprecated ↔ baseline overlap is matrix-independent
        // (deprecated is unconditional). Same for deprecated ↔ quarantine.
        let dir = header_only_dir();
        write(dir.path(), "raikiri-baseline.txt", "css/foo\n");
        write(dir.path(), "deprecated.txt", "css/foo\n");
        // Matrix has zero rows, so it would suppress any baseline ∩
        // quarantine conflict — but that's not the pair here.
        let matrix = RunMatrix::default();
        let now = time::macros::date!(2026 - 07 - 16);
        let report = run_with_matrix(dir.path(), now, Some(&matrix));
        let conflicts: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == Category::Conflicting)
            .collect();
        assert_eq!(conflicts.len(), 1);
    }
}
