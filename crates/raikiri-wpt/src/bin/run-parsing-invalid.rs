//! Report-only sweep: run every discovered `test_invalid_value` WPT parsing
//! test file's assertions and print a PASS/FAIL summary. Never writes to
//! any `expectations/*.txt` file — cross-referencing this output against
//! `expectations/known-issues.txt`/`raikiri-baseline.txt` is a manual step.
//!
//! Usage:
//!
//! ```text
//! cargo run --locked -p raikiri-wpt --bin run-parsing-invalid -- \
//!     --wpt-root target/wpt
//! ```

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use raikiri_wpt::parsing_invalid::{ParsingFileError, run_parsing_invalid_file};

const DEFAULT_WPT_ROOT: &str = "target/wpt";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    wpt_root: PathBuf,
    path_prefix: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            wpt_root: PathBuf::from(DEFAULT_WPT_ROOT),
            path_prefix: None,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let raw_args: Vec<String> = env::args().collect();
    if raw_args
        .iter()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!("{}", usage());
        return Ok(());
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

    let mut total_files = 0usize;
    let mut total_pass_files = 0usize;
    let mut total_assertions = 0usize;
    let mut total_assertions_passed = 0usize;

    for path in &paths {
        let test_id = path_to_string(path);
        if !is_html_like(path) || !has_path_component(&test_id, "parsing") {
            continue;
        }
        if let Some(prefix) = args.path_prefix.as_deref()
            && !test_id.starts_with(prefix)
        {
            continue;
        }
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
    Ok(())
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
    "Usage: run-parsing-invalid [OPTIONS]\n\n  --wpt-root PATH      WPT checkout (default: target/wpt)\n  --path-prefix PATH  Only run files whose path starts with this string\n\nReport-only: never writes to any expectations file."
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
