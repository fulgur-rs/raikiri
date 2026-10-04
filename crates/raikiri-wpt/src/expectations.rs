//! WPT expectations file parsers (spec §12.9, §12.10).
//!
//! 6 files under workspace-root `expectations/`:
//! - `tracked-wpt.txt`      — tracking categories (informational, T3)
//! - `known-issues.txt`     — tests explicitly waived
//! - `raikiri-baseline.txt` — T2 gate (regression blocks merge)
//! - `expected-failures.txt` — deterministic failures that must still run
//! - `quarantine.txt`      — flaky tests, platform-aware
//! - `deprecated.txt`      — fully excluded from evaluation
//!
//! Filtering precedence is deprecated > quarantine > expected failure >
//! known issue. An expected failure is executed and only changes how its
//! actual result is reported; it does not count as a pass.
//!
//! Parsers are `parse(content, file_name)` for unit-testable strings, plus
//! `load(path)` for filesystem I/O. `ExpectationSet::load_from_workspace_root`
//! walks the workspace-root `expectations/` directory.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use time::{Date, macros::format_description};

// ── ExpectationSet ─────────────────────────────────────────────────────────

/// All expectation files combined.
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
    /// Exact deterministic failures that still run and report as XFAIL.
    pub expected_failures: ExpectedFailures,
    /// Flaky tests, platform-aware temporary shelf.
    pub quarantine: Quarantine,
    /// Tests fully excluded from evaluation (highest precedence).
    pub deprecated: Deprecated,
}

impl ExpectationSet {
    /// Load all 6 files from `<workspace root>/expectations/`.
    ///
    /// Workspace root is resolved via `CARGO_MANIFEST_DIR/../..` (this crate
    /// sits at `crates/raikiri-wpt/`).
    pub fn load_from_workspace_root() -> Result<Self, ExpectError> {
        Self::load_from(&workspace_expectations_dir())
    }

    /// Load all 6 files from an arbitrary directory. Used for unit tests
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
        let (expected_failures, expected_failure_errors) =
            ExpectedFailures::load(&dir.join("expected-failures.txt"))?;
        if let Some(error) = expected_failure_errors.into_iter().next() {
            return Err(error);
        }
        let (quarantine, quarantine_errors) = Quarantine::load(&dir.join("quarantine.txt"))?;
        if let Some(e) = quarantine_errors.into_iter().next() {
            return Err(e);
        }
        let deprecated = Deprecated::load(&dir.join("deprecated.txt"))?;
        Ok(Self {
            tracked,
            known_issues,
            baseline,
            expected_failures,
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

// ── ExpectedFailures ──────────────────────────────────────────────────────

/// Parsed `expected-failures.txt`: exact tests whose deterministic failures
/// remain visible as XFAIL until their local Beads issue is resolved.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct ExpectedFailures {
    /// One exact test id per row.
    pub entries: Vec<ExpectedFailure>,
}

/// A single expected-failure record.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExpectedFailure {
    /// Root-relative WPT path, optionally followed by its exact query string.
    pub test_id: String,
    /// Why this deterministic failure is currently accepted.
    pub reason: String,
    /// Local Beads issue tracking the implementation or fixture follow-up.
    pub issue_id: String,
    /// Date this record was added.
    pub added_date: Date,
    /// Date by which the record must be reviewed.
    pub review_by: Date,
    /// 1-based line number in `expected-failures.txt`.
    pub line_no: usize,
}

impl ExpectedFailures {
    /// Parse exact expected-failure rows, accumulating row errors so lint can
    /// report more than one malformed record at a time.
    pub fn parse(content: &str, file_name: &str) -> (Self, Vec<ExpectError>) {
        let mut entries = Vec::new();
        let mut errors = Vec::new();
        for (line_no, line) in iter_data_lines(content) {
            if has_trailing_whitespace(line) {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("trailing whitespace on data line: {line:?}"),
                });
                continue;
            }
            if line.contains('\t') {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: "tab characters are not allowed in expected-failure rows".to_owned(),
                });
                continue;
            }
            let columns: Vec<&str> = line.split('|').map(str::trim).collect();
            if columns.len() != 5 {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("expected 5 pipe-delimited columns, got {}", columns.len()),
                });
                continue;
            }
            let test_id = columns[0];
            let reason = columns[1];
            let issue_id = columns[2];
            let added = columns[3];
            let review_by = columns[4];
            if !is_exact_test_id(test_id) {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!(
                        "expected one exact root-relative WPT test id, got {test_id:?}"
                    ),
                });
                continue;
            }
            if reason.is_empty() {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: "reason must not be empty".to_owned(),
                });
                continue;
            }
            if !is_local_beads_id(issue_id) {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: format!("expected a local raikiri-spike Beads id, got {issue_id:?}"),
                });
                continue;
            }
            let parse_date = |value: &str, column: &str| {
                Date::parse(value, &format_description!("[year]-[month]-[day]")).map_err(|error| {
                    ExpectError::MalformedLine {
                        file: file_name.to_owned(),
                        line_no,
                        reason: format!("{column} {value:?} is not YYYY-MM-DD ({error})"),
                    }
                })
            };
            let added_date = match parse_date(added, "added_date") {
                Ok(date) => date,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            let review_date = match parse_date(review_by, "review_by") {
                Ok(date) => date,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            if review_date <= added_date {
                errors.push(ExpectError::MalformedLine {
                    file: file_name.to_owned(),
                    line_no,
                    reason: "review_by must be later than added_date".to_owned(),
                });
                continue;
            }
            entries.push(ExpectedFailure {
                test_id: test_id.to_owned(),
                reason: reason.to_owned(),
                issue_id: issue_id.to_owned(),
                added_date,
                review_by: review_date,
                line_no,
            });
        }
        (Self { entries }, errors)
    }

    /// Read and parse `expected-failures.txt` from `path`.
    pub fn load(path: &Path) -> Result<(Self, Vec<ExpectError>), ExpectError> {
        let content = read_file(path)?;
        Ok(Self::parse(&content, &path.display().to_string()))
    }

    /// Find an exact test id. Directory prefixes and query normalization are
    /// deliberately not applied.
    pub fn get(&self, test_id: &str) -> Option<&ExpectedFailure> {
        self.entries.iter().find(|entry| entry.test_id == test_id)
    }

    /// True if no expected failures were parsed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn is_exact_test_id(test_id: &str) -> bool {
    if test_id.is_empty()
        || test_id.starts_with('/')
        || test_id.contains('\\')
        || test_id.contains('#')
        || test_id.contains('*')
        || test_id.chars().any(char::is_whitespace)
    {
        return false;
    }
    let (path, query) = test_id
        .split_once('?')
        .map_or((test_id, None), |(path, query)| (path, Some(query)));
    if path.is_empty()
        || path.ends_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return false;
    }
    query.is_none_or(|query| !query.is_empty() && !query.contains('?'))
}

fn is_local_beads_id(issue_id: &str) -> bool {
    issue_id
        .strip_prefix("raikiri-spike-")
        .is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '-')
                })
        })
}

// ── Baseline (§12.10, T2 gate) ─────────────────────────────────────────────

/// Parsed `raikiri-baseline.txt`: the T2 gate set (spec §12.10).
///
/// Regressions against this set block merge.
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
mod tests;
