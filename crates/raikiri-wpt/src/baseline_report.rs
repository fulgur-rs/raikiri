//! Per-test report over the WPT baseline.
//!
//! Runs every test id in a baseline file through the harness, records one
//! status per id, and diffs two reports. It is report-only: nothing here
//! decides whether a build passes.

use crate::parsing_invalid::{ParsingFileError, run_parsing_invalid_file};
use crate::reftest::{
    ReftestConfig, ReftestError, ReftestKind, ReftestPair, ReftestResult,
    discover_pairs_for_file_with_wpt_root, run_pair, run_pair_with_images,
};
use crate::runner::TestOutcome;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Result of running one baseline id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every reference pair (or every assertion) passed.
    Pass,
    /// The harness ran and reported a mismatch or a failed assertion.
    Fail,
    /// The harness could not run the test (missing file, panic, harness error).
    Error,
    /// The harness skipped the test.
    Skip,
}

impl Status {
    /// The name written to a report file.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::Error => "ERROR",
            Status::Skip => "SKIP",
        }
    }

    /// Inverse of [`Status::as_str`].
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "PASS" => Some(Status::Pass),
            "FAIL" => Some(Status::Fail),
            "ERROR" => Some(Status::Error),
            "SKIP" => Some(Status::Skip),
            _ => None,
        }
    }
}

/// One line of a report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Test id, relative to the WPT root.
    pub id: String,
    /// Outcome.
    pub status: Status,
    /// Free-form reason for a non-pass status; empty for a pass.
    pub detail: String,
}

/// Detail recorded on a PASS that only held with local resources disabled.
///
/// The pixel comparison is then between a page and its reference with images
/// left out, which is a weaker claim than a full comparison; [`diff`] reports a
/// PASS that lost its clean status as weakened.
pub const FALLBACK_NOTE: &str = "passes only without local resources";

/// Test ids listed in a baseline file (one per line, `#` starts a comment).
pub fn parse_baseline_ids(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// Whether an id is a `css/**/parsing/*` page, run by the testharness runner.
pub fn is_parsing_id(id: &str) -> bool {
    id.starts_with("css/") && id.split('/').any(|part| part == "parsing")
}

/// Serialize rows as `id<TAB>STATUS<TAB>detail` lines.
pub fn write_tsv(rows: &[Row]) -> String {
    let mut out = String::new();
    for row in rows {
        let detail: String = row
            .detail
            .chars()
            .map(|c| {
                if matches!(c, '\t' | '\n' | '\r') {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            row.id,
            row.status.as_str(),
            detail
        ));
    }
    out
}

/// Parse the output of [`write_tsv`].
pub fn parse_tsv(text: &str) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let id = parts.next().unwrap_or_default();
        let status = parts
            .next()
            .and_then(Status::parse)
            .ok_or_else(|| format!("line {}: missing or unknown status", index + 1))?;
        rows.push(Row {
            id: id.to_owned(),
            status,
            detail: parts.next().unwrap_or("").to_owned(),
        });
    }
    Ok(rows)
}

/// Differences between two reports, by id.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// PASS before, anything else after.
    pub regressions: Vec<String>,
    /// Not PASS before, PASS after.
    pub fixes: Vec<String>,
    /// Present before, absent after.
    pub missing: Vec<String>,
    /// PASS both times, but only after falling back to a run without local
    /// resources (see [`FALLBACK_NOTE`]) where the first report was clean.
    pub weakened: Vec<String>,
}

/// Compare `before` with `after`.
pub fn diff(before: &[Row], after: &[Row]) -> Diff {
    let after_by_id: BTreeMap<&str, &Row> =
        after.iter().map(|row| (row.id.as_str(), row)).collect();
    let mut result = Diff::default();
    for row in before {
        let Some(now) = after_by_id.get(row.id.as_str()) else {
            result.missing.push(row.id.clone());
            continue;
        };
        match (row.status, now.status) {
            (Status::Pass, Status::Pass) => {
                if row.detail != FALLBACK_NOTE && now.detail == FALLBACK_NOTE {
                    result.weakened.push(row.id.clone());
                }
            }
            (Status::Pass, _) => result.regressions.push(row.id.clone()),
            (_, Status::Pass) => result.fixes.push(row.id.clone()),
            _ => {}
        }
    }
    result
}

/// Human-readable summary of a [`Diff`].
pub fn render_diff(diff: &Diff) -> String {
    let mut out = String::new();
    for (title, ids) in [
        ("regressions", &diff.regressions),
        ("fixes", &diff.fixes),
        ("missing", &diff.missing),
        ("weakened", &diff.weakened),
    ] {
        out.push_str(&format!("{title}: {}\n", ids.len()));
        for id in ids {
            out.push_str(&format!("  {id}\n"));
        }
    }
    out
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned())
}

/// Run `body`, turning a panic into an [`Status::Error`] row.
fn catch_row(id: &str, body: impl FnOnce() -> (Status, String)) -> Row {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok((status, detail)) => Row {
            id: id.to_owned(),
            status,
            detail,
        },
        Err(payload) => Row {
            id: id.to_owned(),
            status: Status::Error,
            detail: format!("panic: {}", panic_message(payload.as_ref())),
        },
    }
}

fn reftest_status(wpt_root: &Path, id: &str) -> (Status, String) {
    let test = wpt_root.join(id);
    let pairs = match discover_pairs_for_file_with_wpt_root(&test, Some(wpt_root)) {
        Ok(pairs) => pairs,
        Err(error) => return (Status::Error, format!("discover: {error}")),
    };
    if pairs.is_empty() {
        return (Status::Error, "no reference pair".to_owned());
    }
    let config = ReftestConfig::default();
    let results = pairs
        .iter()
        .map(|pair| {
            let (status, detail) = pair_status(pair, config);
            (pair.kind, status, detail)
        })
        .collect();
    combine_pairs(results)
}

fn outcome_status(result: Result<ReftestResult, ReftestError>) -> (Status, String) {
    match result {
        Ok(result) => match result.outcome {
            TestOutcome::Pass => (Status::Pass, String::new()),
            TestOutcome::Fail(reason) => (
                Status::Fail,
                format!("{reason} ({} px)", result.mismatched_pixels),
            ),
            TestOutcome::Skip(reason) => (Status::Skip, reason),
            TestOutcome::Quarantined => (Status::Skip, "quarantined".to_owned()),
        },
        Err(error) => (Status::Error, format!("run: {error}")),
    }
}

/// One reference pair passes when it passes with local resources enabled or
/// without them: the baseline mixes tests pinned under each mode. A pass that
/// needed the resource-free run is marked with [`FALLBACK_NOTE`]. When neither
/// passes, the resource-enabled result is reported.
fn pair_status(pair: &ReftestPair, config: ReftestConfig) -> (Status, String) {
    let with_resources = outcome_status(run_pair_with_images(pair, config));
    if with_resources.0 == Status::Pass {
        return with_resources;
    }
    let plain = outcome_status(run_pair(pair, config));
    if plain.0 == Status::Pass {
        (Status::Pass, FALLBACK_NOTE.to_owned())
    } else {
        with_resources
    }
}

/// Combine per-reference results the way WPT does: `rel=match` references are
/// alternatives (one of them must match) and every `rel=mismatch` reference
/// must differ. On failure the first non-pass result of the deciding group is
/// reported.
fn combine_pairs(results: Vec<(ReftestKind, Status, String)>) -> (Status, String) {
    let of_kind = |kind: ReftestKind| {
        results
            .iter()
            .filter(move |(candidate, ..)| *candidate == kind)
            .collect::<Vec<_>>()
    };
    let matches = of_kind(ReftestKind::Match);
    let mismatches = of_kind(ReftestKind::Mismatch);
    let match_ok = matches.is_empty() || matches.iter().any(|(_, s, _)| *s == Status::Pass);
    let mismatch_ok = mismatches.iter().all(|(_, s, _)| *s == Status::Pass);
    if match_ok && mismatch_ok {
        // Prefer a matching alternative that needed no fallback; the note is
        // kept when the pass rests on a fallback run.
        let clean_match = matches
            .iter()
            .any(|(_, s, detail)| *s == Status::Pass && detail.is_empty());
        let fell_back = (!matches.is_empty() && !clean_match)
            || mismatches.iter().any(|(_, _, detail)| !detail.is_empty());
        let detail = if fell_back { FALLBACK_NOTE } else { "" };
        return (Status::Pass, detail.to_owned());
    }
    let deciding = if match_ok { &mismatches } else { &matches };
    deciding
        .iter()
        .find(|(_, status, _)| *status != Status::Pass)
        .map(|(_, status, detail)| (*status, detail.clone()))
        .unwrap_or((Status::Fail, "no reference matched".to_owned()))
}

fn parsing_status(wpt_root: &Path, id: &str) -> (Status, String) {
    match run_parsing_invalid_file(wpt_root, Path::new(id)) {
        Ok(outcome) if outcome.all_passed() => (Status::Pass, String::new()),
        Ok(outcome) => (
            Status::Fail,
            format!("{}/{} assertions passed", outcome.passed(), outcome.total()),
        ),
        Err(ParsingFileError::NoParsingTestCalls) => {
            (Status::Skip, "no parsing test calls".to_owned())
        }
        Err(error) => (Status::Error, error.to_string()),
    }
}

/// Run one baseline id with the runner that matches its kind.
///
/// A reftest passes at 800x600 with an exact pixel match, which is how the
/// baseline defines PASS: one `rel=match` reference must match (they are
/// alternatives) and every `rel=mismatch` reference must differ. The
/// known-issues prefix filter is deliberately not applied: it overlaps the
/// baseline.
pub fn run_id(wpt_root: &Path, id: &str) -> Row {
    // Reference lookup and dynamic reftests need an absolute root.
    let wpt_root = std::fs::canonicalize(wpt_root).unwrap_or_else(|_| wpt_root.to_path_buf());
    let wpt_root = wpt_root.as_path();
    if is_parsing_id(id) {
        catch_row(id, || parsing_status(wpt_root, id))
    } else {
        catch_row(id, || reftest_status(wpt_root, id))
    }
}

/// Run `ids` on `jobs` threads with `run`, calling `on_row` as each finishes.
/// Rows come back in input order.
pub fn run_ids_with<F>(
    ids: &[String],
    jobs: usize,
    run: F,
    on_row: &(dyn Fn(&Row) + Sync),
) -> Vec<Row>
where
    F: Fn(&str) -> Row + Sync,
{
    let jobs = jobs.clamp(1, ids.len().max(1));
    if jobs == 1 {
        return ids
            .iter()
            .map(|id| {
                let row = run(id);
                on_row(&row);
                row
            })
            .collect();
    }
    let next = AtomicUsize::new(0);
    let slots: Mutex<Vec<Option<Row>>> = Mutex::new(vec![None; ids.len()]);
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(id) = ids.get(index) else { break };
                    let row = run(id);
                    on_row(&row);
                    slots.lock().unwrap_or_else(|e| e.into_inner())[index] = Some(row);
                }
            });
        }
    });
    slots
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .map(|slot| slot.expect("every index is claimed exactly once"))
        .collect()
}

/// [`run_ids_with`] over the real harness.
pub fn run_ids(
    wpt_root: &Path,
    ids: &[String],
    jobs: usize,
    on_row: &(dyn Fn(&Row) + Sync),
) -> Vec<Row> {
    run_ids_with(ids, jobs, |id| run_id(wpt_root, id), on_row)
}

/// Options of `run-baseline-report`'s report mode.
#[derive(Debug, PartialEq, Eq)]
pub struct ReportOptions {
    /// Root of the fetched WPT checkout.
    pub wpt_root: PathBuf,
    /// Baseline file listing the ids to run.
    pub baseline: PathBuf,
    /// Report file to write; standard output when absent.
    pub output: Option<PathBuf>,
    /// Worker threads.
    pub jobs: usize,
    /// Run only these ids instead of the whole baseline.
    pub only: Vec<String>,
    /// Stop after this many ids.
    pub limit: Option<usize>,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            wpt_root: PathBuf::from("target/wpt"),
            baseline: PathBuf::from("expectations/raikiri-baseline.txt"),
            output: None,
            jobs: 1,
            only: Vec::new(),
            limit: None,
        }
    }
}

fn flag_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

/// Parse the arguments of report mode.
pub fn parse_report_args(args: &[String]) -> Result<ReportOptions, String> {
    let mut options = ReportOptions::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        index += 1;
        match flag {
            "--wpt-root" => options.wpt_root = PathBuf::from(flag_value(args, index, flag)?),
            "--baseline" => options.baseline = PathBuf::from(flag_value(args, index, flag)?),
            "--output" => options.output = Some(PathBuf::from(flag_value(args, index, flag)?)),
            "--only" => options.only.push(flag_value(args, index, flag)?.to_owned()),
            "--jobs" => {
                let jobs: usize = flag_value(args, index, flag)?
                    .parse()
                    .map_err(|_| "--jobs expects a number".to_owned())?;
                if jobs == 0 {
                    return Err("--jobs must be at least 1".to_owned());
                }
                options.jobs = jobs;
            }
            "--limit" => {
                options.limit = Some(
                    flag_value(args, index, flag)?
                        .parse()
                        .map_err(|_| "--limit expects a number".to_owned())?,
                );
            }
            other => return Err(format!("unrecognized argument: {other}")),
        }
        index += 1;
    }
    Ok(options)
}

#[cfg(test)]
mod tests;
