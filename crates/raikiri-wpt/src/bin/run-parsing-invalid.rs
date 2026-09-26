//! Report-only sweep: run every discovered `test_invalid_value` WPT parsing
//! test file's assertions and print a PASS/FAIL summary. Never writes to
//! any `expectations/*.txt` file — cross-referencing this output against
//! `expectations/known-issues.txt`/`raikiri-baseline.txt` is a manual step.
//!
//! Usage:
//!
//! ```text
//! cargo run --locked -p raikiri-wpt --bin run-parsing-invalid -- \
//!     --wpt-root target/wpt [--path-prefix PREFIX] [--compare]
//! ```
//!
//! `--compare` runs every file on both the legacy shim and the native DOM
//! runtime, prints files whose `test_invalid_value` results differ, and
//! exits 1 if the native runtime does worse on any file.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use raikiri_wpt::parsing_invalid::{
    Engine, ParsingFileError, ParsingFileOutcome, compare, run_parsing_invalid_file,
    run_parsing_invalid_file_with,
};

const DEFAULT_WPT_ROOT: &str = "target/wpt";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    wpt_root: PathBuf,
    path_prefix: Option<String>,
    compare: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            wpt_root: PathBuf::from(DEFAULT_WPT_ROOT),
            path_prefix: None,
            compare: false,
        }
    }
}

fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    }
}

/// Returns `Ok(false)` when `--compare` found a regression.
fn run() -> Result<bool, String> {
    let raw_args: Vec<String> = env::args().collect();
    if raw_args
        .iter()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!("{}", usage());
        return Ok(true);
    }
    let args = parse_args(&raw_args[1..])?;

    let mut paths = Vec::new();
    collect_files(&args.wpt_root.join("css"), &args.wpt_root, &mut paths).map_err(|e| {
        format!(
            "could not enumerate {}: {e}",
            args.wpt_root.join("css").display()
        )
    })?;
    paths.sort();
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .filter(|path| {
            let test_id = path_to_string(path);
            is_html_like(path)
                && has_path_component(&test_id, "parsing")
                && args
                    .path_prefix
                    .as_deref()
                    .is_none_or(|prefix| test_id.starts_with(prefix))
        })
        .collect();

    if args.compare {
        return Ok(compare_engines(&args.wpt_root, &paths));
    }

    let mut total_files = 0usize;
    let mut total_pass_files = 0usize;
    let mut total_assertions = 0usize;
    let mut total_assertions_passed = 0usize;

    for path in &paths {
        let test_id = path_to_string(path);
        match run_parsing_invalid_file(&args.wpt_root, path) {
            Ok(outcome) => {
                total_files += 1;
                total_assertions += outcome.total();
                total_assertions_passed += outcome.passed();
                if outcome.all_passed() {
                    total_pass_files += 1;
                }
                println!(
                    "{} {}/{} {}",
                    if outcome.all_passed() { "PASS" } else { "FAIL" },
                    outcome.passed(),
                    outcome.total(),
                    outcome.test_id,
                );
                if !outcome.all_passed() {
                    for o in &outcome.outcomes {
                        if !o.passed {
                            println!("    FAIL: {} ({})", o.name, o.message);
                        }
                    }
                }
            }
            Err(ParsingFileError::NoInvalidValueCalls) => continue,
            Err(e) => {
                total_files += 1;
                println!("ERROR {test_id}: {e}");
            }
        }
    }

    println!(
        "-- {total_pass_files}/{total_files} files all-pass, \
         {total_assertions_passed}/{total_assertions} assertions pass --"
    );
    Ok(true)
}

type FileResult = Result<ParsingFileOutcome, ParsingFileError>;

/// Run every file on both engines and report `test_invalid_value`
/// differences. Returns `false` when the native engine regressed any file.
fn compare_engines(wpt_root: &Path, paths: &[PathBuf]) -> bool {
    let mut rows: Vec<(String, FileResult, FileResult)> = Vec::new();
    for path in paths {
        let legacy = run_parsing_invalid_file_with(wpt_root, path, Engine::Legacy);
        let native = run_parsing_invalid_file_with(wpt_root, path, Engine::Native);
        if matches!(legacy, Err(ParsingFileError::NoInvalidValueCalls))
            && matches!(native, Err(ParsingFileError::NoInvalidValueCalls))
        {
            continue;
        }
        rows.push((path_to_string(path), legacy, native));
    }
    let comparison = compare(
        rows.iter()
            .map(|(test_id, legacy, native)| (test_id.as_str(), legacy, native)),
    );
    for difference in &comparison.regressions {
        println!(
            "REGRESSION {}: legacy {}, native {}",
            difference.test_id, difference.legacy, difference.native
        );
    }
    for difference in &comparison.improvements {
        println!(
            "IMPROVED {}: legacy {}, native {}",
            difference.test_id, difference.legacy, difference.native
        );
    }
    println!(
        "-- compare: {} regressions, {} improvements, {} files --",
        comparison.regressions.len(),
        comparison.improvements.len(),
        rows.len()
    );
    println!(
        "-- legacy: {} --",
        totals(rows.iter().map(|(_, legacy, _)| legacy))
    );
    println!(
        "-- native: {} --",
        totals(rows.iter().map(|(_, _, native)| native))
    );
    comparison.regressions.is_empty()
}

/// `test_invalid_value` and all-outcome totals plus the error count.
fn totals<'a>(results: impl Iterator<Item = &'a FileResult>) -> String {
    let (mut invalid_passed, mut invalid_total, mut passed, mut total, mut errors) =
        (0, 0, 0, 0, 0);
    for result in results {
        match result {
            Ok(outcome) => {
                invalid_passed += outcome.invalid_value_passed();
                invalid_total += outcome.invalid_value_total();
                passed += outcome.passed();
                total += outcome.total();
            }
            Err(_) => errors += 1,
        }
    }
    format!(
        "test_invalid_value {invalid_passed}/{invalid_total} pass, \
         all outcomes {passed}/{total} pass, {errors} file errors"
    )
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut result = Args::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--wpt-root" => {
                i += 1;
                let value = args.get(i).ok_or("--wpt-root requires a value")?;
                result.wpt_root = PathBuf::from(value);
            }
            "--compare" => result.compare = true,
            "--path-prefix" => {
                i += 1;
                let value = args.get(i).ok_or("--path-prefix requires a value")?;
                result.path_prefix = Some(value.clone());
            }
            other => return Err(format!("unrecognized argument: {other}")),
        }
        i += 1;
    }
    Ok(result)
}

fn usage() -> &'static str {
    "Usage: run-parsing-invalid [OPTIONS]\n\n  --wpt-root PATH      WPT checkout (default: target/wpt)\n  --path-prefix PATH  Only run files whose path starts with this string\n  --compare           Run the legacy shim and the native runtime and exit 1\n                      if any file regresses on the native runtime\n\nReport-only: never writes to any expectations file."
}

fn collect_files(directory: &Path, root: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(&path, root, files)?;
        } else if entry.file_type()?.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("file is below WPT root")
                .to_path_buf();
            files.push(relative);
        }
    }
    Ok(())
}

fn is_html_like(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("html" | "htm" | "xhtml" | "xht")
    )
}

fn has_path_component(path: &str, component: &str) -> bool {
    path.split('/').any(|part| part == component)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
