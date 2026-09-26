//! Report-only runner for testharness pages in `css/css-text/i18n`.

use std::path::PathBuf;
use std::process::ExitCode;

use raikiri_wpt::text_css_i18n::{TestHarnessRunError, run_css_text_i18n};

fn main() -> ExitCode {
    let Options { wpt_root } = match parse_args() {
        Ok(options) => options,
        Err(ParseArgsError::Help) => {
            println!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Err(ParseArgsError::Invalid(message)) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    let results = match run_css_text_i18n(&wpt_root) {
        Ok(results) => results,
        Err(error) => return run_failed(&error),
    };

    let mut passed_files = 0usize;
    let mut passed_assertions = 0usize;
    let mut total_assertions = 0usize;
    let mut failed_assertions = 0usize;
    let mut errors = 0usize;

    for result in &results {
        if let Some(error) = &result.error {
            errors += 1;
            eprintln!("ERROR {}: {error}", result.test_id);
            continue;
        }

        let passed = result.passed();
        let total = result.total();
        passed_files += usize::from(result.all_passed());
        passed_assertions += passed;
        total_assertions += total;
        failed_assertions += total - passed;
        let label = if result.all_passed() { "PASS" } else { "FAIL" };
        println!("{label} {passed}/{total} {}", result.test_id);
        for outcome in result.outcomes.iter().filter(|outcome| !outcome.passed) {
            println!("  {}: {}", outcome.name, outcome.message);
        }
    }

    println!(
        "-- {passed_files}/{} files all-pass, {passed_assertions}/{total_assertions} assertions pass, {failed_assertions} assertion failures, {errors} execution errors --",
        results.len()
    );

    if failed_assertions > 0 || errors > 0 || passed_files != results.len() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn run_failed(error: &TestHarnessRunError) -> ExitCode {
    eprintln!("WPT testharness run failed: {error}");
    ExitCode::from(2)
}

struct Options {
    wpt_root: PathBuf,
}

enum ParseArgsError {
    Help,
    Invalid(String),
}

fn parse_args() -> Result<Options, ParseArgsError> {
    let mut args = std::env::args().skip(1);
    let mut wpt_root = PathBuf::from("target/wpt");
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--wpt-root" => {
                wpt_root = PathBuf::from(args.next().ok_or_else(|| {
                    ParseArgsError::Invalid("--wpt-root requires a path".to_owned())
                })?);
            }
            "--help" | "-h" => return Err(ParseArgsError::Help),
            unknown => {
                return Err(ParseArgsError::Invalid(format!(
                    "unknown argument: {unknown}\n{}",
                    usage()
                )));
            }
        }
    }
    Ok(Options { wpt_root })
}

fn usage() -> &'static str {
    "Usage: run-css-text-i18n [--wpt-root PATH]\n\nRuns testharness-only pages under css/css-text/i18n.\nReport-only: does not modify expectations files."
}
