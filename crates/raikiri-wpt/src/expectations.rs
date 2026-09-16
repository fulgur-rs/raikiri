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
/// // baseline is runner-generated once a future runner task lands (see raikiri-baseline.txt
/// // header). quarantine/deprecated stay empty until a developer PR
/// // adds an entry — flakes for quarantine (§12.10 procedure) and
/// // crashers for deprecated. tracked/known_issues are populated
/// // statically from spec §12.9 (see expectations/*.txt).
/// assert!(set.baseline.is_empty());
/// assert!(set.quarantine.entries.is_empty());
/// assert!(set.deprecated.entries.is_empty());
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
    ///
    /// Multiple row-level quarantine errors are collapsed to the first
    /// (subsequent row errors are discarded). Callers that need every
    /// row error surfaced (lint pass) should use
    /// [`crate::lint::run`] instead, which reads the raw file and calls
    /// [`Quarantine::parse`] directly to preserve the full error vector.
    pub fn load_from(dir: &Path) -> Result<Self, ExpectError> {
        let tracked = TrackedWpt::load(&dir.join("tracked-wpt.txt"))?;
        let known_issues = KnownIssues::load(&dir.join("known-issues.txt"))?;
        let baseline = Baseline::load(&dir.join("raikiri-baseline.txt"))?;
        let (quarantine, quarantine_errors) = Quarantine::load(&dir.join("quarantine.txt"))?;
        if let Some(e) = quarantine_errors.into_iter().next() {
            return Err(e);
        }
        let deprecated = Deprecated::load(&dir.join("deprecated.txt"))?;
        Ok(Self {
            tracked,
            known_issues,
            baseline,
            quarantine,
            deprecated,
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

/// Yield `(line_no, line)` for non-comment, non-blank data lines.
///
/// The returned `line` is stripped of leading whitespace only. Callers
/// that treat trailing whitespace as [`ExpectError::MalformedLine`]
/// (spec §12.10) compare `line == line.trim_end()`. Comment and blank
/// lines are still filtered using a fully-trimmed view so
/// `   # comment` and `   \n` are ignored.
fn iter_data_lines(content: &str) -> impl Iterator<Item = (usize, &str)> {
    content.lines().enumerate().filter_map(|(i, line)| {
        let fully_trimmed = line.trim();
        if fully_trimmed.is_empty() || fully_trimmed.starts_with('#') {
            None
        } else {
            Some((i + 1, line.trim_start()))
        }
    })
}

/// True if `line` has any trailing ASCII whitespace ($space$ / tab / CR).
///
/// Callers use this to surface [`ExpectError::MalformedLine`] for lines
/// like `"css/foo   "` per spec §12.10 (unexpected characters).
fn has_trailing_whitespace(line: &str) -> bool {
    line.len() != line.trim_end().len()
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
                .map(|(_, l)| l.trim_end().to_owned())
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
    ///
    /// A data line that contains `|` or trailing whitespace surfaces as
    /// [`ExpectError::MalformedLine`] (spec §12.10): baseline has a
    /// single-column shape, and a pipe strongly suggests the row was
    /// copied from `quarantine.txt`.
    pub fn parse(content: &str, file_name: &str) -> Result<Self, ExpectError> {
        let mut entries = HashSet::new();
        for (line_no, raw) in iter_data_lines(content) {
            if raw.contains('|') {
                return Err(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!(
                        "expected single-column test id, found pipe delimiter: {:?}",
                        raw.trim_end()
                    ),
                });
            }
            if has_trailing_whitespace(raw) {
                return Err(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("trailing whitespace on data line: {raw:?}"),
                });
            }
            entries.insert(raw.to_owned());
        }
        Ok(Self { entries })
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
    ///
    /// A data line that contains `|` or trailing whitespace surfaces as
    /// [`ExpectError::MalformedLine`] (spec §12.10): deprecated has a
    /// single-column shape, and a pipe strongly suggests the row was
    /// copied from `quarantine.txt`.
    pub fn parse(content: &str, file_name: &str) -> Result<Self, ExpectError> {
        let mut entries = HashSet::new();
        for (line_no, raw) in iter_data_lines(content) {
            if raw.contains('|') {
                return Err(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!(
                        "expected single-column test id, found pipe delimiter: {:?}",
                        raw.trim_end()
                    ),
                });
            }
            if has_trailing_whitespace(raw) {
                return Err(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("trailing whitespace on data line: {raw:?}"),
                });
            }
            entries.insert(raw.to_owned());
        }
        Ok(Self { entries })
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
    /// [`time::Date::parse`]; a malformed date is recorded in the
    /// `Vec<ExpectError>` returned alongside the parsed [`Quarantine`] as
    /// [`ExpectError::MalformedLine`], and the offending row is skipped.
    pub added_date: Date,
    /// 1-based line number in `quarantine.txt` where this entry was parsed from.
    pub line_no: usize,
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
    /// Parse a `quarantine.txt` platform column value (`"linux"`,
    /// `"macos"`, `"windows"`, or `"*"` for [`Self::Any`]). Returns
    /// `None` for unrecognized input. Public so `raikiri_wpt::lint`
    /// callers can parse env-var-supplied matrix rows against the same
    /// enum grammar.
    pub fn parse_str(s: &str) -> Option<Self> {
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
    /// Parse a `quarantine.txt` arch column value (`"x86_64"`,
    /// `"aarch64"`, or `"*"` for [`Self::Any`]). Returns `None` for
    /// unrecognized input. See [`PlatformFilter::parse_str`] for the
    /// rationale for exposing this as public API.
    pub fn parse_str(s: &str) -> Option<Self> {
        Some(match s {
            "x86_64" => Self::X86_64,
            "aarch64" => Self::Aarch64,
            "*" => Self::Any,
            _ => return None,
        })
    }
}

impl RendererFilter {
    /// Parse a `quarantine.txt` renderer column value (`"vello_cpu"`,
    /// `"skia"`, `"tiny_skia"`, or `"*"` for [`Self::Any`]). Returns
    /// `None` for unrecognized input. See [`PlatformFilter::parse_str`]
    /// for the rationale for exposing this as public API.
    pub fn parse_str(s: &str) -> Option<Self> {
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
    /// Parse a `quarantine.txt` tolerance column value (`"pixel-exact"`,
    /// `"low"`, `"medium"`, `"high"`, or `"*"` for [`Self::Any`]).
    /// Returns `None` for unrecognized input. See
    /// [`PlatformFilter::parse_str`] for the rationale for exposing
    /// this as public API.
    pub fn parse_str(s: &str) -> Option<Self> {
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
    ///
    /// Row-level validation errors (bad column count, unknown enum values,
    /// malformed `added_date`) are accumulated in the returned
    /// `Vec<ExpectError>` and the offending rows are skipped; other valid
    /// rows are still surfaced in the returned [`Quarantine`]. This lets
    /// the lint pass surface every issue in a file at once instead of
    /// aborting on the first malformed row.
    pub fn parse(content: &str, file_name: &str) -> (Self, Vec<ExpectError>) {
        let mut entries = Vec::new();
        let mut errors: Vec<ExpectError> = Vec::new();
        for (line_no, line) in iter_data_lines(content) {
            if has_trailing_whitespace(line) {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("trailing whitespace on data line: {line:?}"),
                });
                continue;
            }
            let cols: Vec<&str> = line.split('|').map(str::trim).collect();
            if cols.len() != 8 {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("expected 8 pipe-delimited columns, got {}", cols.len()),
                });
                continue;
            }
            let Some(platform) = PlatformFilter::parse_str(cols[1]) else {
                errors.push(ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "platform",
                    value: cols[1].to_owned(),
                });
                continue;
            };
            let Some(arch) = ArchFilter::parse_str(cols[2]) else {
                errors.push(ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "arch",
                    value: cols[2].to_owned(),
                });
                continue;
            };
            let Some(renderer) = RendererFilter::parse_str(cols[3]) else {
                errors.push(ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "renderer",
                    value: cols[3].to_owned(),
                });
                continue;
            };
            let Some(tolerance) = ToleranceFilter::parse_str(cols[4]) else {
                errors.push(ExpectError::UnknownEnum {
                    file: file_name.to_owned(),
                    line_no,
                    field: "tolerance",
                    value: cols[4].to_owned(),
                });
                continue;
            };
            let added_date =
                match Date::parse(cols[7], &format_description!("[year]-[month]-[day]")) {
                    Ok(d) => d,
                    Err(e) => {
                        errors.push(ExpectError::MalformedLine {
                            file: file_name.to_owned(),
                            line_no,
                            reason: format!("added_date {:?} is not YYYY-MM-DD ({e})", cols[7]),
                        });
                        continue;
                    }
                };
            entries.push(QuarantineEntry {
                test_id: cols[0].to_owned(),
                platform,
                arch,
                renderer,
                tolerance,
                reason: cols[5].to_owned(),
                issue_link: cols[6].to_owned(),
                added_date,
                line_no,
            });
        }
        (Self { entries }, errors)
    }

    /// Read and parse `quarantine.txt` from `path`. The outer [`Result`]
    /// surfaces filesystem I/O errors only; per-row parse errors are
    /// returned in the inner tuple's `Vec<ExpectError>`.
    pub fn load(path: &Path) -> Result<(Self, Vec<ExpectError>), ExpectError> {
        let content = read_file(path)?;
        Ok(Self::parse(&content, &path.display().to_string()))
    }

    /// True if no entries were parsed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── ExpectError (hand-written Display + Error + From) ────────

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

        let (q, q_errs) = Quarantine::parse("# 8-col format follows\n", "quarantine.txt");
        assert!(q_errs.is_empty());
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
    fn baseline_rejects_pipe_delimiter() {
        // Row copy-pasted from quarantine.txt should surface as Malformed
        // rather than pass silently as a bogus test id.
        let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n";
        let err = Baseline::parse(content, "raikiri-baseline.txt").unwrap_err();
        match err {
            ExpectError::MalformedLine {
                file,
                line_no,
                reason,
            } => {
                assert_eq!(file, "raikiri-baseline.txt");
                assert_eq!(line_no, 1);
                assert!(reason.contains("pipe delimiter"), "got: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn baseline_rejects_trailing_whitespace() {
        let content = "css/foo/bar-001   \n";
        let err = Baseline::parse(content, "raikiri-baseline.txt").unwrap_err();
        match err {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(line_no, 1);
                assert!(reason.contains("trailing whitespace"), "got: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn deprecated_rejects_pipe_delimiter() {
        let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01\n";
        let err = Deprecated::parse(content, "deprecated.txt").unwrap_err();
        match err {
            ExpectError::MalformedLine {
                file,
                line_no,
                reason,
            } => {
                assert_eq!(file, "deprecated.txt");
                assert_eq!(line_no, 1);
                assert!(reason.contains("pipe delimiter"), "got: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn deprecated_rejects_trailing_whitespace() {
        let content = "css/foo/bar-001\t\n";
        let err = Deprecated::parse(content, "deprecated.txt").unwrap_err();
        match err {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(line_no, 1);
                assert!(reason.contains("trailing whitespace"), "got: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn baseline_and_deprecated_accept_comment_and_blank_lines() {
        let content = "# header\n\ncss/foo/bar-001\n\n# trailing comment\n";
        let base = Baseline::parse(content, "b.txt").unwrap();
        assert_eq!(base.entries.len(), 1);
        let dep = Deprecated::parse(content, "d.txt").unwrap();
        assert_eq!(dep.entries.len(), 1);
    }

    #[test]
    fn quarantine_parses_8_cols_and_enum_values() {
        let content = "css/css-page/page-margin-boxes-001 | macos | aarch64 | vello_cpu | pixel-exact | Intermittent 1-pixel diff | https://example/issues/123 | 2026-08-01\n";
        let (q, errs) = Quarantine::parse(content, "q.txt");
        assert!(errs.is_empty(), "expected no errors, got {errs:?}");
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
        let (q, errs) = Quarantine::parse(content, "q.txt");
        assert!(errs.is_empty(), "expected no errors, got {errs:?}");
        let e = &q.entries[0];
        assert_eq!(e.platform, PlatformFilter::Any);
        assert_eq!(e.arch, ArchFilter::Any);
        assert_eq!(e.renderer, RendererFilter::Any);
        assert_eq!(e.tolerance, ToleranceFilter::Any);
    }

    #[test]
    fn quarantine_rejects_wrong_col_count() {
        let content = "css/foo | linux | x86_64\n"; // 3 cols
        let (q, errs) = Quarantine::parse(content, "q.txt");
        assert!(q.entries.is_empty());
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(*line_no, 1);
                assert!(reason.contains("expected 8"));
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_rejects_unknown_platform() {
        let content = "css/foo | plan9 | x86_64 | vello_cpu | low | r | i | 2026-08-01\n";
        let (q, errs) = Quarantine::parse(content, "q.txt");
        assert!(q.entries.is_empty());
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ExpectError::UnknownEnum { field, value, .. } => {
                assert_eq!(*field, "platform");
                assert_eq!(value, "plan9");
            }
            other => panic!("expected UnknownEnum, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_rejects_malformed_added_date() {
        let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | not-a-date\n";
        let (q, errs) = Quarantine::parse(content, "q.txt");
        assert!(q.entries.is_empty());
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(*line_no, 1);
                assert!(reason.contains("added_date"), "got: {reason}");
                assert!(reason.contains("not-a-date"), "got: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_rejects_calendar_invalid_added_date_feb30() {
        // 2026-02-30 is syntactically YYYY-MM-DD but no such calendar day exists.
        let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-02-30\n";
        let (q, errs) = Quarantine::parse(content, "q.txt");
        assert!(q.entries.is_empty());
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(*line_no, 1);
                assert!(reason.contains("added_date"), "got: {reason}");
                assert!(reason.contains("2026-02-30"), "got: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_rejects_calendar_invalid_added_date_month13() {
        // Month 13 is syntactically valid but out of range.
        let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-13-01\n";
        let (q, errs) = Quarantine::parse(content, "q.txt");
        assert!(q.entries.is_empty());
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(*line_no, 1);
                assert!(reason.contains("added_date"), "got: {reason}");
                assert!(reason.contains("2026-13-01"), "got: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_accepts_leap_year_feb_29() {
        // 2024 is a leap year, so 2024-02-29 is a valid calendar day.
        let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2024-02-29\n";
        let (q, errs) = Quarantine::parse(content, "q.txt");
        assert!(errs.is_empty(), "expected no errors, got {errs:?}");
        assert_eq!(q.entries.len(), 1);
        assert_eq!(q.entries[0].added_date, time::macros::date!(2024 - 02 - 29));
    }

    #[test]
    fn quarantine_rejects_trailing_whitespace() {
        // Trailing whitespace on a data row surfaces as MalformedLine
        // (spec §12.10) even when the column count is right.
        let content = "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01   \n";
        let (q, errs) = Quarantine::parse(content, "q.txt");
        assert!(
            q.entries.is_empty(),
            "expected no entries, got {:?}",
            q.entries
        );
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(*line_no, 1);
                assert!(reason.contains("trailing whitespace"), "got: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_accumulates_multiple_row_errors() {
        // 3 rows: valid, malformed (wrong column count), valid.
        // Expect the malformed row to accumulate as an error while both
        // valid rows survive as entries.
        let content = "\
css/a | linux | x86_64 | vello_cpu | low | r | i | 2026-08-01
css/bad | linux | x86_64
css/b | macos | aarch64 | skia | high | r | i | 2026-08-02
";
        let (q, errors) = Quarantine::parse(content, "q.txt");
        assert_eq!(
            q.entries.len(),
            2,
            "expected 2 valid entries, got {:?}",
            q.entries
        );
        assert_eq!(q.entries[0].test_id, "css/a");
        assert_eq!(q.entries[0].line_no, 1);
        assert_eq!(q.entries[1].test_id, "css/b");
        assert_eq!(q.entries[1].line_no, 3);
        assert_eq!(errors.len(), 1);
        match &errors[0] {
            ExpectError::MalformedLine {
                line_no, reason, ..
            } => {
                assert_eq!(*line_no, 2);
                assert!(reason.contains("expected 8"), "got reason: {reason}");
            }
            other => panic!("expected MalformedLine, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_continues_after_enum_error() {
        // Bad platform followed by a valid row. Ensure the valid row is
        // still surfaced in entries and the enum error is captured.
        let content = "\
css/bad | plan9 | x86_64 | vello_cpu | low | r | i | 2026-08-01
css/ok | macos | aarch64 | skia | high | r | i | 2026-08-02
";
        let (q, errors) = Quarantine::parse(content, "q.txt");
        assert_eq!(q.entries.len(), 1);
        assert_eq!(q.entries[0].test_id, "css/ok");
        assert_eq!(errors.len(), 1);
        match &errors[0] {
            ExpectError::UnknownEnum {
                field,
                value,
                line_no,
                ..
            } => {
                assert_eq!(*field, "platform");
                assert_eq!(value, "plan9");
                assert_eq!(*line_no, 1);
            }
            other => panic!("expected UnknownEnum, got {other:?}"),
        }
    }

    #[test]
    fn quarantine_records_only_first_column_error_per_row() {
        // A row with two invalid columns (bad platform AND bad arch) must
        // produce exactly one error — the first-failing column (platform) —
        // per the short-circuit-per-row contract. If a future refactor
        // switches to intra-row accumulation this test will fail loudly.
        let content = "css/x | plan9 | notarch | vello_cpu | low | r | i | 2026-08-01\n";
        let (q, errors) = Quarantine::parse(content, "q.txt");
        assert!(q.entries.is_empty());
        assert_eq!(errors.len(), 1);
        match &errors[0] {
            ExpectError::UnknownEnum { field, value, .. } => {
                assert_eq!(*field, "platform");
                assert_eq!(value, "plan9");
            }
            other => panic!("expected UnknownEnum(platform), got {other:?}"),
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
        // Integration-style: verify each expectations/*.txt against the
        // full expected content (spec §12.9 tracked categories + §2 Non-Goals
        // waivers). Count-only assertions cannot detect typos, duplicates, or
        // ordering drift — assert exact membership to make transcription bugs
        // observable.
        let set = ExpectationSet::load_from_workspace_root()
            .expect("workspace expectations/ should be present");

        let tracked: Vec<&str> = set.tracked.entries.iter().map(String::as_str).collect();
        assert_eq!(
            tracked,
            vec![
                // P1 — Foundation
                "css/css-fonts/",
                "css/css-color/",
                "css/css-backgrounds/",
                "css/css-values/",
                "css/css-text/",
                "css/css-text-decor/",
                "css/css-writing-modes/",
                "css/selectors/",
                "html/rendering/",
                // P2 — Layout primitives
                "css/css-tables/",
                "css/css-grid/",
                "css/css-flexbox/",
                // P3 — GCPM / paged media
                "css/css-page/",
                "css/css-break/",
                // P4 — Low priority
                "css/css-transforms/",
            ],
        );

        let known_issues: Vec<(&str, &str)> = set
            .known_issues
            .entries
            .iter()
            .map(|(pat, reason)| (pat.as_str(), reason.as_str()))
            .collect();
        // Check that all expected baseline entries are present (allow additional quarantines)
        let expected_baseline = vec![
            (
                "css/css-animations/",
                "Non-goal (interactive, §2 Non-Goals + §12.9)",
            ),
            (
                "css/css-transitions/",
                "Non-goal (interactive, §2 Non-Goals + §12.9)",
            ),
            (
                "html/interaction/",
                "Non-goal (interactive rendering, §2 Non-Goals + §12.9)",
            ),
            (
                "css/css-ruby/",
                "Non-goal for MVP (JIS X 4051 / 縦書き outside MVP scope, §2 Non-Goals)",
            ),
            (
                "accname/",
                "Non-goal (Consumer builds a11y tree from hints, §2 Non-Goals)",
            ),
            (
                "wai-aria/",
                "Non-goal (Consumer builds a11y tree from hints, §2 Non-Goals)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-blend-mode.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-descendants.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-ellipsis.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-flex.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-fragmentation.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-inline-block-child.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-inline.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-multiline-background-image.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-multiline-linebreak.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-on-body-not-propagated-to-root.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-on-body-scroll.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-out-of-flow-child.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-relative-child.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-scaled.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-stacking-context-child.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-text-align.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-text-decorations.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-text-text-emphasis.html",
                "background-clip:text glyph-outline clipping not implemented (CSS Backgrounds 4 §2.6, requires skrifa→BezPath→push_clip_layer)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-border-area-border-on-top.html",
                "background-clip multi-layer (border-area, border-box) with background-image list not implemented (single-layer scope carving)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-border-area-multiple-backgrounds.html",
                "background-clip multi-layer (border-area, border-box, content-box) with background-image list not implemented",
            ),
            (
                "css/css-backgrounds/background-clip/clip-border-area-text.html",
                "background-clip border-area text union (CSS Backgrounds 4 §2.6) requires glyph + border-area compositing not implemented",
            ),
            (
                "css/css-backgrounds/background-clip/clip-border-area-border-shape-background-position.html",
                "border-shape inset(0) background-position interaction not implemented (CSS Borders 4)",
            ),
            (
                "css/css-backgrounds/background-clip/clip-border-shape-table-part-background.html",
                "table-part background with border-shape not implemented",
            ),
        ];
        // Allow additional entries beyond baseline (goal B quarantines)
        for exp in &expected_baseline {
            assert!(
                known_issues.contains(exp),
                "expected known-issue {:?} missing",
                exp
            );
        }

        // baseline populated via chore(wpt): pin PASS file-level parsing tests + goal G (79) + selectors (10) (WPT 97ea26e)
        // + grid reftest H (+20) + text reftest retry2 J2 (+176) + flexbox reftest retry2 I2 (+191)
        // + css-tables reftest sweep (+29) + table-layout/border-collapse parsing-valid (+2)
        // + css-tables parsing-8 (+7: border-spacing/caption-side/empty-cells valid + caption-side/empty-cells/table-layout/border-collapse computed)
        // + css-tables reftest tables-sweep (+4: zero-rowspan-001, row-group-order, percent-height-replaced-in-percent-cell.tentative, table-cell-baseline-static-position).
        // + css-text-decor parsing jqg8 (+10)
        // + page-valid + flex-basis-valid + flex-invalid parsing tgag (+3).
        // + segment-break removable/ignorable exact reftests (+5).
        // + text-transform capitalize whitespace/abspos exact reftest (+1).
        // - hyphens-punctuation-001 (-1, 9q1p: container-width re-break exposed
        //   a false pass; genuine pass needs hyphenation dictionaries, bd u94v).
        // + text-align end-001..008 (sans 009/010) + start-001..008 + start-010
        //   (+17, dir-attribute UA rules: explicit-direction and dir=ltr/rtl/auto
        //   cases now resolve; 009/010 need zero-width RLM handling, follow-up).
        // - bidi-tab-001 (-1: dir rules made direction real, exposing
        //   direction-naive tab-stop expansion in RTL spans; needs bidi-aware
        //   tab stops, follow-up).
        // + subpixel-table-cell-width-001/002 (+2, glk7 grid text-skip:
        //   non-element children carry no grid structure).
        // + segment-break-transformation-ignorable/removable x5 + text-transform-capitalize-034
        //   (+6, css-text-whitespace #10: multi-node runs, ignorable neighbors, abspos inline).
        // + CSS Text PASS-only WPT sweeps (+25: white-space/text-wrap,
        //   word-break/overflow-wrap, text-indent, tab-size).
        // + text-align-last paint-time final-line alignment (+16).
        // + letter-spacing bridge and hyphens PASS-only sweep (+6).
        // quarantine and deprecated stay empty until a developer PR adds a flake or crasher.
        // + flexbox_fbfc (+1, y0zo initial slice: exact 800x600 pass).
        // + flexbox-overflow-vert-003 (+1, y0zo overflow clip: exact 800x600 pass).
        // + flexbox gap/wrapping slice (+8, exact 800x600 pass).
        // + grid item-sizing slice (+2, exact 800x600 pass).
        // + table flex percentage width (+1, exact 800x600 pass).
        // + abspos static-position slice (+9, exact 800x600 pass).
        // - inline-flex percentage sizing: one flex-item pin restored after
        //   the Taffy calc resolver was connected; one remains deferred.
        assert_eq!(set.baseline.entries.len(), 940);
        assert!(set.quarantine.is_empty());
        assert!(set.deprecated.is_empty());
    }
}
