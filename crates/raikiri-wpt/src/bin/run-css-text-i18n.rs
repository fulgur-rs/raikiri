//! Report-only runner for testharness pages in `css/css-text/i18n`.

use std::path::PathBuf;
use std::process::ExitCode;

use raikiri_wpt::text_css_i18n::{
    Engine, TestHarnessFileResult, TestHarnessRunError, compare, run_css_text_i18n,
    run_css_text_i18n_with,
};

fn main() -> ExitCode {
    let Options { wpt_root, compare } = match parse_args() {
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

    if compare {
        return compare_engines(&wpt_root);
    }

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

/// Run every page on both engines, print the files whose results differ,
/// and exit 1 when the real harness regressed any file.
fn compare_engines(wpt_root: &std::path::Path) -> ExitCode {
    let shim = match run_css_text_i18n_with(wpt_root, Engine::Shim) {
        Ok(results) => results,
        Err(error) => return run_failed(&error),
    };
    let real = match run_css_text_i18n_with(wpt_root, Engine::Real) {
        Ok(results) => results,
        Err(error) => return run_failed(&error),
    };
    let comparison = compare(shim.iter().zip(real.iter()));
    for difference in &comparison.regressions {
        println!(
            "REGRESSION {}: shim {}, real {}",
            difference.test_id, difference.shim, difference.real
        );
    }
    for difference in &comparison.improvements {
        println!(
            "IMPROVED {}: shim {}, real {}",
            difference.test_id, difference.shim, difference.real
        );
    }
    println!(
        "-- compare: {} regressions, {} improvements, {} files --",
        comparison.regressions.len(),
        comparison.improvements.len(),
        shim.len()
    );
    println!("-- shim: {} --", totals(&shim));
    println!("-- real: {} --", totals(&real));
    if comparison.regressions.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Assertion totals plus the error count.
fn totals(results: &[TestHarnessFileResult]) -> String {
    let passed: usize = results.iter().map(TestHarnessFileResult::passed).sum();
    let total: usize = results.iter().map(TestHarnessFileResult::total).sum();
    let all_pass = results.iter().filter(|result| result.all_passed()).count();
    let errors = results
        .iter()
        .filter(|result| result.error.is_some())
        .count();
    format!(
        "{all_pass}/{} files all-pass, {passed}/{total} assertions pass, {errors} execution errors",
        results.len()
    )
}

fn run_failed(error: &TestHarnessRunError) -> ExitCode {
    eprintln!("WPT testharness run failed: {error}");
    ExitCode::from(2)
}

struct Options {
    wpt_root: PathBuf,
    compare: bool,
}

enum ParseArgsError {
    Help,
    Invalid(String),
}

fn parse_args() -> Result<Options, ParseArgsError> {
    let mut args = std::env::args().skip(1);
    let mut wpt_root = PathBuf::from("target/wpt");
    let mut compare = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--wpt-root" => {
                wpt_root = PathBuf::from(args.next().ok_or_else(|| {
                    ParseArgsError::Invalid("--wpt-root requires a path".to_owned())
                })?);
            }
            "--compare" => compare = true,
            "--help" | "-h" => return Err(ParseArgsError::Help),
            unknown => {
                return Err(ParseArgsError::Invalid(format!(
                    "unknown argument: {unknown}\n{}",
                    usage()
                )));
            }
        }
    }
    Ok(Options { wpt_root, compare })
}

fn usage() -> &'static str {
    "Usage: run-css-text-i18n [--wpt-root PATH] [--compare]\n\nRuns testharness-only pages under css/css-text/i18n.\n--compare runs every page on the shim and the real testharness.js and\nexits 1 if any page regresses on the real harness.\nReport-only: does not modify expectations files."
}
