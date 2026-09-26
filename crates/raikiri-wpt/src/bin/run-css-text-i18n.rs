//! Report-only runner for testharness pages in `css/css-text/i18n`.

use std::path::PathBuf;
use std::process::ExitCode;

use raikiri_wpt::text_css_i18n::{
    Engine, TestHarnessFileResult, TestHarnessRunError, regressions, run_css_text_i18n,
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
        return match compare_engines(&wpt_root) {
            Ok(code) => code,
            Err(error) => run_failed(&error),
        };
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

fn run_failed(error: &TestHarnessRunError) -> ExitCode {
    eprintln!("WPT testharness run failed: {error}");
    ExitCode::from(2)
}

/// Run every page on both engines and report files where the native engine
/// does worse (exit 1) or better than the legacy one.
fn compare_engines(wpt_root: &std::path::Path) -> Result<ExitCode, TestHarnessRunError> {
    let legacy = run_css_text_i18n_with(wpt_root, Engine::Legacy)?;
    let native = run_css_text_i18n_with(wpt_root, Engine::Native)?;
    let found = regressions(&legacy, &native);
    for regression in &found {
        println!(
            "REGRESSION {}: legacy {}, native {}",
            regression.test_id, regression.legacy, regression.native
        );
    }
    // Swapping the arguments lists files where legacy did worse; its
    // `legacy` field then holds the native summary and vice versa.
    let improved = regressions(&native, &legacy);
    for improvement in &improved {
        println!(
            "IMPROVED {}: legacy {}, native {}",
            improvement.test_id, improvement.native, improvement.legacy
        );
    }
    println!(
        "-- compare: {} regressions, {} improvements, {} files; legacy {}, native {} --",
        found.len(),
        improved.len(),
        native.len(),
        totals(&legacy),
        totals(&native)
    );
    Ok(if found.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn totals(results: &[TestHarnessFileResult]) -> String {
    let passed: usize = results.iter().map(TestHarnessFileResult::passed).sum();
    let total: usize = results.iter().map(TestHarnessFileResult::total).sum();
    let errors = results
        .iter()
        .filter(|result| result.error.is_some())
        .count();
    format!("{passed}/{total} assertions pass, {errors} execution errors")
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
    "Usage: run-css-text-i18n [--wpt-root PATH] [--compare]\n\nRuns testharness-only pages under css/css-text/i18n.\nReport-only: does not modify expectations files.\n\n--compare  run every page on the legacy and native DOM engines and\n           exit 1 if any file does worse on the native engine."
}
