//! Report-only runner for testharness pages in `css/css-text/i18n`.

use raikiri_wpt::testharness_results::{ResultRecord, extract_output, write_results};
use std::path::PathBuf;
use std::process::ExitCode;

use raikiri_wpt::text_css_i18n::{TestHarnessRunError, run_css_text_i18n};

fn main() -> ExitCode {
    let Options {
        wpt_root,
        results_path,
    } = match parse_args() {
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

    let records: Vec<_> = results
        .iter()
        .map(|r| ResultRecord::new(&r.test_id, &r.outcomes, r.error.clone()))
        .collect();
    if let Err(error) = write_results(results_path.as_deref(), &records) {
        eprintln!("result output: {error}");
        return ExitCode::from(2);
    }
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
    results_path: Option<PathBuf>,
}

enum ParseArgsError {
    Help,
    Invalid(String),
}

fn parse_args() -> Result<Options, ParseArgsError> {
    let mut raw: Vec<String> = std::env::args().collect();
    let results_path = extract_output(&mut raw).map_err(ParseArgsError::Invalid)?;
    let mut args = raw.into_iter().skip(1);
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
    Ok(Options {
        wpt_root,
        results_path,
    })
}

fn usage() -> &'static str {
    "Usage: run-css-text-i18n [--wpt-root PATH] [--results-json PATH]\n\nRuns testharness pages under css/css-text/i18n with the real testharness.js.\nReport-only: does not modify expectations files."
}
