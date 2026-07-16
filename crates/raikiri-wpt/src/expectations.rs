//! WPT expectations file parsers (spec §12.9, §12.10).
//!
//! 5 files under workspace-root `expectations/`:
//! - `tracked-wpt.txt`     — tracking categories (informational, T3)
//! - `known-issues.txt`    — tests explicitly waived
//! - `raikiri-baseline.txt` — T2 gate (regression blocks merge)
//! - `quarantine.txt`      — flaky tests, platform-aware
//! - `deprecated.txt`      — fully excluded from evaluation
//!
//! Precedence (highest → lowest): deprecated > quarantine > baseline > default.
//!
//! Parsers are `parse(content, file_name)` for unit-testable strings, plus
//! `load(path)` for filesystem I/O. `ExpectationSet::load_from_workspace_root`
//! walks the workspace-root `expectations/` directory.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use time::{Date, macros::format_description};

// ── ExpectationSet ─────────────────────────────────────────────────────────

/// All expectations files combined.
///
/// ```no_run
/// use raikiri_wpt::expectations::ExpectationSet;
/// let set = ExpectationSet::load_from_workspace_root().unwrap();
/// assert!(set.baseline.is_empty()); // M1 skeleton: workspace files are header-only
/// ```
#[non_exhaustive]
pub struct ExpectationSet {
    /// Tracking categories (informational, T3).
    pub tracked: TrackedWpt,
    /// Tests explicitly waived as "cannot pass, no practical impact".
    pub known_issues: KnownIssues,
    /// T2 gate: tests raikiri currently passes; regressions block merge.
    pub baseline: Baseline,
    /// Flaky tests, platform-aware temporary shelf.
    pub quarantine: Quarantine,
    /// Tests fully excluded from evaluation (highest precedence).
    pub deprecated: Deprecated,
}

impl ExpectationSet {
    /// Load all 5 files from `<workspace root>/expectations/`.
    ///
    /// Workspace root is resolved via `CARGO_MANIFEST_DIR/../..` (this crate
    /// sits at `crates/raikiri-wpt/`).
    pub fn load_from_workspace_root() -> Result<Self, ExpectError> {
        Self::load_from(&workspace_expectations_dir())
    }

    /// Load all 5 files from an arbitrary directory. Used for unit tests
    /// via `parse(&str, &str)` on individual files and for future
    /// integration tests that stage fixture directories.
    pub fn load_from(dir: &Path) -> Result<Self, ExpectError> {
        Ok(Self {
            tracked: TrackedWpt::load(&dir.join("tracked-wpt.txt"))?,
            known_issues: KnownIssues::load(&dir.join("known-issues.txt"))?,
            baseline: Baseline::load(&dir.join("raikiri-baseline.txt"))?,
            quarantine: Quarantine::load(&dir.join("quarantine.txt"))?,
            deprecated: Deprecated::load(&dir.join("deprecated.txt"))?,
        })
    }
}

fn workspace_expectations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("expectations")
}

// ── Parser helpers ─────────────────────────────────────────────────────────

fn iter_data_lines(content: &str) -> impl Iterator<Item = (usize, &str)> {
    content
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
}

fn read_file(path: &Path) -> Result<String, ExpectError> {
    std::fs::read_to_string(path).map_err(ExpectError::Io)
}

// ── TrackedWpt (§12.9) ─────────────────────────────────────────────────────

/// Parsed `tracked-wpt.txt`: tracking categories (informational, T3).
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TrackedWpt {
    /// One test id per non-comment, non-empty line.
    pub entries: Vec<String>,
}

impl TrackedWpt {
    /// Parse `tracked-wpt.txt` content. `_file_name` is unused (no
    /// per-line format to fail); kept for API symmetry with the other
    /// parsers.
    pub fn parse(content: &str, _file_name: &str) -> Result<Self, ExpectError> {
        Ok(Self {
            entries: iter_data_lines(content)
                .map(|(_, l)| l.to_owned())
                .collect(),
        })
    }

    /// Read and parse `tracked-wpt.txt` from `path`.
    pub fn load(path: &Path) -> Result<Self, ExpectError> {
        let content = read_file(path)?;
        Self::parse(&content, &path.display().to_string())
    }

    /// True if no entries were parsed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── KnownIssues (§12.9) ────────────────────────────────────────────────────

/// Parsed `known-issues.txt`: tests explicitly waived (spec §12.9).
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct KnownIssues {
    /// `(pattern, reason)` pairs.
    pub entries: Vec<(String, String)>,
}

impl KnownIssues {
    /// Parse `known-issues.txt` content (`pattern | reason` per line).
    pub fn parse(content: &str, file_name: &str) -> Result<Self, ExpectError> {
        let mut entries = Vec::new();
        for (line_no, line) in iter_data_lines(content) {
            let mut parts = line.splitn(2, '|').map(str::trim);
            match (parts.next(), parts.next()) {
                (Some(pat), Some(reason)) if !pat.is_empty() && !reason.is_empty() => {
                    entries.push((pat.to_owned(), reason.to_owned()));
                }
                _ => {
                    return Err(ExpectError::MalformedLine {
                        file: file_name.to_owned(),
                        line_no,
                        reason: "expected `test_id_or_pattern | reason`".to_owned(),
                    });
                }
            }
        }
        Ok(Self { entries })
    }

    /// Read and parse `known-issues.txt` from `path`.
    pub fn load(path: &Path) -> Result<Self, ExpectError> {
        let content = read_file(path)?;
        Self::parse(&content, &path.display().to_string())
    }

    /// True if no entries were parsed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── Baseline (§12.10, T2 gate) ─────────────────────────────────────────────

/// Parsed `raikiri-baseline.txt`: the T2 gate set (spec §12.10).
///
/// Regressions against this set block merge; additions/removals require
/// 2-reviewer approval per the baseline-migration policy.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct Baseline {
    /// Deduplicated set of test ids in the T2 baseline.
    pub entries: HashSet<String>,
}

impl Baseline {
    /// Parse `raikiri-baseline.txt` content (one test id per line).
    pub fn parse(content: &str, _file_name: &str) -> Result<Self, ExpectError> {
        Ok(Self {
            entries: iter_data_lines(content)
                .map(|(_, l)| l.to_owned())
                .collect(),
        })
    }

    /// Read and parse `raikiri-baseline.txt` from `path`.
    pub fn load(path: &Path) -> Result<Self, ExpectError> {
        let content = read_file(path)?;
        Self::parse(&content, &path.display().to_string())
    }

    /// True if no entries were parsed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── Deprecated (§12.10, precedence 1) ──────────────────────────────────────

/// Parsed `deprecated.txt`: tests fully excluded from evaluation
/// (spec §12.10, highest precedence).
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct Deprecated {
    /// Deduplicated set of deprecated test ids.
    pub entries: HashSet<String>,
}

impl Deprecated {
    /// Parse `deprecated.txt` content (one test id per line).
    pub fn parse(content: &str, _file_name: &str) -> Result<Self, ExpectError> {
        Ok(Self {
            entries: iter_data_lines(content)
                .map(|(_, l)| l.to_owned())
                .collect(),
        })
    }

    /// Read and parse `deprecated.txt` from `path`.
    pub fn load(path: &Path) -> Result<Self, ExpectError> {
        let content = read_file(path)?;
        Self::parse(&content, &path.display().to_string())
    }

    /// True if no entries were parsed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── Quarantine (§12.10, 8-col platform-aware) ──────────────────────────────

/// Parsed `quarantine.txt`: flaky tests, platform-aware (spec §12.10).
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct Quarantine {
    /// One entry per quarantined test/platform-filter combination.
    pub entries: Vec<QuarantineEntry>,
}

/// A single quarantine rule: one 8-column pipe-delimited row of
/// `quarantine.txt`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QuarantineEntry {
    /// WPT test id this rule applies to.
    pub test_id: String,
    /// OS platform this rule is scoped to (or [`PlatformFilter::Any`]).
    pub platform: PlatformFilter,
    /// CPU architecture this rule is scoped to (or [`ArchFilter::Any`]).
    pub arch: ArchFilter,
    /// Renderer backend this rule is scoped to (or [`RendererFilter::Any`]).
    pub renderer: RendererFilter,
    /// Pixel-diff tolerance this rule is scoped to (or
    /// [`ToleranceFilter::Any`]).
    pub tolerance: ToleranceFilter,
    /// Human-readable reason for the quarantine.
    pub reason: String,
    /// Tracking issue URL for the flake.
    pub issue_link: String,
    /// Date the entry was added, in YYYY-MM-DD (ISO 8601) form. Parsed and
    /// validated at [`Quarantine::parse`] time via
    /// [`time::Date::parse`]; a malformed date surfaces as
    /// [`ExpectError::MalformedLine`].
    pub added_date: Date,
}

/// OS platform column of a [`QuarantineEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlatformFilter {
    /// Linux only.
    Linux,
    /// macOS only.
    MacOs,
    /// Windows only.
    Windows,
    /// Any platform (`*`).
    Any,
}

/// CPU architecture column of a [`QuarantineEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArchFilter {
    /// x86_64 only.
    X86_64,
    /// aarch64 only.
    Aarch64,
    /// Any architecture (`*`).
    Any,
}

/// Renderer backend column of a [`QuarantineEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RendererFilter {
    /// `vello_cpu` renderer only.
    VelloCpu,
    /// `skia` renderer only.
    Skia,
    /// `tiny_skia` renderer only.
    TinySkia,
    /// Any renderer (`*`).
    Any,
}

/// Pixel-diff tolerance column of a [`QuarantineEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToleranceFilter {
    /// Exact pixel match required.
    PixelExact,
    /// Low tolerance.
    Low,
    /// Medium tolerance.
    Medium,
    /// High tolerance.
    High,
    /// Any tolerance (`*`).
    Any,
}

impl PlatformFilter {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "linux" => Self::Linux,
            "macos" => Self::MacOs,
            "windows" => Self::Windows,
            "*" => Self::Any,
            _ => return None,
        })
    }
}

impl ArchFilter {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "x86_64" => Self::X86_64,
            "aarch64" => Self::Aarch64,
            "*" => Self::Any,
            _ => return None,
        })
    }
}

impl RendererFilter {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "vello_cpu" => Self::VelloCpu,
            "skia" => Self::Skia,
            "tiny_skia" => Self::TinySkia,
            "*" => Self::Any,
            _ => return None,
        })
    }
}

impl ToleranceFilter {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pixel-exact" => Self::PixelExact,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "*" => Self::Any,
            _ => return None,
        })
    }
}

impl Quarantine {
    /// Parse `quarantine.txt` content (8 pipe-delimited columns per line).
    pub fn parse(content: &str, file_name: &str) -> Result<Self, ExpectError> {
        let mut entries = Vec::new();
        for (line_no, line) in iter_data_lines(content) {
            let cols: Vec<&str> = line.split('|').map(str::trim).collect();
            if cols.len() != 8 {
                return Err(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("expected 8 pipe-delimited columns, got {}", cols.len()),
                });
            }
            let platform =
                PlatformFilter::parse(cols[1]).ok_or_else(|| ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "platform",
                    value: cols[1].to_owned(),
                })?;
            let arch = ArchFilter::parse(cols[2]).ok_or_else(|| ExpectError::UnknownEnum {
                file: file_name.to_owned(),
                line_no,
                field: "arch",
                value: cols[2].to_owned(),
            })?;
            let renderer =
                RendererFilter::parse(cols[3]).ok_or_else(|| ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "renderer",
                    value: cols[3].to_owned(),
                })?;
            let tolerance =
                ToleranceFilter::parse(cols[4]).ok_or_else(|| ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "tolerance",
                    value: cols[4].to_owned(),
                })?;
            let added_date_fmt = format_description!("[year]-[month]-[day]");
            let added_date = Date::parse(cols[7], &added_date_fmt).map_err(|e| {
                ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("added_date {:?} is not YYYY-MM-DD ({e})", cols[7]),
                }
            })?;
            entries.push(QuarantineEntry {
                test_id: cols[0].to_owned(),
                platform,
                arch,
                renderer,
                tolerance,
                reason: cols[5].to_owned(),
                issue_link: cols[6].to_owned(),
                added_date,
            });
        }
        Ok(Self { entries })
    }

    /// Read and parse `quarantine.txt` from `path`.
    pub fn load(path: &Path) -> Result<Self, ExpectError> {
        let content = read_file(path)?;
        Self::parse(&content, &path.display().to_string())
    }

    /// True if no entries were parsed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── ExpectError (m1.2 pattern: hand-written Display + Error + From) ────────

/// Errors from parsing or loading expectations files.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExpectError {
    /// Underlying filesystem I/O error.
    Io(std::io::Error),
    /// A line failed the shape expected by the file format.
    MalformedLine {
        /// File the malformed line was read from (path or logical name).
        file: String,
        /// 1-based line number of the malformed line.
        line_no: usize,
        /// Human-readable description of what was expected.
        reason: String,
    },
    /// A pipe-delimited column carried a value not in the accepted enum.
    UnknownEnum {
        /// File the offending line was read from (path or logical name).
        file: String,
        /// 1-based line number of the offending line.
        line_no: usize,
        /// Name of the column/field that failed to parse.
        field: &'static str,
        /// The raw value that did not match any known enum variant.
        value: String,
    },
}

impl fmt::Display for ExpectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "expectations I/O error: {e}"),
            Self::MalformedLine {
                file,
                line_no,
                reason,
            } => {
                write!(f, "{file}:{line_no}: malformed line ({reason})")
            }
            Self::UnknownEnum {
                file,
                line_no,
                field,
                value,
            } => {
                write!(f, "{file}:{line_no}: unknown {field} value {value:?}")
            }
        }
    }
}

impl std::error::Error for ExpectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ExpectError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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

        let q = Quarantine::parse("# 8-col format follows\n", "quarantine.txt").unwrap();
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
    fn quarantine_parses_8_cols_and_enum_values() {
        let content = "css/css-page/page-margin-boxes-001 | macos | aarch64 | vello_cpu | pixel-exact | Intermittent 1-pixel diff | https://example/issues/123 | 2026-08-01\n";
        let q = Quarantine::parse(content, "q.txt").unwrap();
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
        let q = Quarantine::parse(content, "q.txt").unwrap();
        let e = &q.entries[0];
        assert_eq!(e.platform, PlatformFilter::Any);
        assert_eq!(e.arch, ArchFilter::Any);
        assert_eq!(e.renderer, RendererFilter::Any);
        assert_eq!(e.tolerance, ToleranceFilter::Any);
    }

    #[test]
    fn quarantine_rejects_wrong_col_count() {
        let content = "css/foo | linux | x86_64\n"; // 3 cols
        let err = Quarantine::parse(content, "q.txt").unwrap_err();
        match err {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(line_no, 1);
                assert!(reason.contains("expected 8"));
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_rejects_unknown_platform() {
        let content = "css/foo | plan9 | x86_64 | vello_cpu | low | r | i | 2026-08-01\n";
        let err = Quarantine::parse(content, "q.txt").unwrap_err();
        match err {
            ExpectError::UnknownEnum { field, value, .. } => {
                assert_eq!(field, "platform");
                assert_eq!(value, "plan9");
            }
            other => panic!("expected UnknownEnum, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_rejects_malformed_added_date() {
        let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | not-a-date\n";
        let err = Quarantine::parse(content, "q.txt").unwrap_err();
        match err {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(line_no, 1);
                assert!(reason.contains("added_date"), "got: {reason}");
                assert!(reason.contains("not-a-date"), "got: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
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
        // Integration-style: relies on Task 1 having created the workspace
        // expectations/ directory with header-only files.
        let set = ExpectationSet::load_from_workspace_root()
            .expect("workspace expectations/ should be present");
        assert!(set.tracked.is_empty());
        assert!(set.known_issues.is_empty());
        assert!(set.baseline.is_empty());
        assert!(set.quarantine.is_empty());
        assert!(set.deprecated.is_empty());
    }
}
