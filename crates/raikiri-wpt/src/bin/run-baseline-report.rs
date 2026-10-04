//! Report-only run of the WPT baseline.
//!
//! ```text
//! cargo run --locked -p raikiri-wpt --bin run-baseline-report -- \
//!     --wpt-root target/wpt --output target/baseline-report/before.tsv --jobs 4
//! cargo run --locked -p raikiri-wpt --bin run-baseline-report -- \
//!     diff target/baseline-report/before.tsv target/baseline-report/after.tsv
//! ```
//!
//! Report mode writes one `id<TAB>STATUS<TAB>detail` line per baseline and
//! expected-failure id. `diff` lists the ids that were PASS in the first
//! report and are not in the second (regressions), the ids that became PASS
//! (fixes), and the ids missing from the second report. `--strict` also makes
//! FAIL, XPASS, and ERROR results fail the command.

use raikiri_wpt::baseline_report::{
    Row, Status, diff, parse_baseline_ids, parse_report_args, parse_tsv, render_diff,
    run_ids_with_expected_failures, write_tsv,
};
use raikiri_wpt::expectations::ExpectedFailures;

const USAGE: &str = "usage:\n  run-baseline-report [--wpt-root DIR] [--baseline FILE] [--output FILE] [--jobs N] [--only ID]... [--limit N] [--strict] [--wpt-fonts]\n  --strict fails on FAIL, XPASS, and ERROR; recorded XFAIL remains visible and accepted\n  (text is laid out with the WPT fonts; --wpt-fonts names that default)\n  run-baseline-report diff BEFORE.tsv AFTER.tsv";

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--help" | "-h") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("diff") => run_diff(&args[1..]),
        Some(_) => run_report(&args),
    }
}

fn run_report(args: &[String]) -> Result<(), String> {
    let options = parse_report_args(args)?;
    let mut ids = if options.only.is_empty() {
        let text = std::fs::read_to_string(&options.baseline)
            .map_err(|e| format!("read {}: {e}", options.baseline.display()))?;
        parse_baseline_ids(&text)
    } else {
        options.only.clone()
    };
    let expected_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../expectations/expected-failures.txt");
    let (expected_failures, parse_errors) = ExpectedFailures::load(&expected_path)
        .map_err(|error| format!("read {}: {error}", expected_path.display()))?;
    if let Some(error) = parse_errors.first() {
        return Err(format!("invalid {}: {error}", expected_path.display()));
    }
    if options.only.is_empty() {
        let mut seen: std::collections::HashSet<String> = ids.iter().cloned().collect();
        for entry in &expected_failures.entries {
            if seen.insert(entry.test_id.clone()) {
                ids.push(entry.test_id.clone());
            }
        }
    }
    if let Some(limit) = options.limit {
        ids.truncate(limit);
    }
    // A run that silently fell back to the installed fonts would give
    // machine-dependent numbers, so refuse to start without the WPT fonts.
    raikiri_wpt::reftest::check_inline_formatting_fonts()
        .map_err(|e| format!("inline engine fonts: {e}"))?;
    eprintln!("running {} tests with {} job(s)", ids.len(), options.jobs);
    let rows = run_ids_with_expected_failures(
        &options.wpt_root,
        &ids,
        &expected_failures,
        options.jobs,
        &|row: &Row| {
            if row.status != Status::Pass {
                eprintln!("{}\t{}\t{}", row.status.as_str(), row.id, row.detail);
            }
        },
    );
    let text = write_tsv(&rows);
    match &options.output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create {}: {e}", parent.display()))?;
            }
            std::fs::write(path, &text).map_err(|e| format!("write {}: {e}", path.display()))?;
        }
        None => print!("{text}"),
    }
    let passed = rows.iter().filter(|r| r.status == Status::Pass).count();
    let xfailed = rows.iter().filter(|r| r.status == Status::XFail).count();
    let xpassed = rows.iter().filter(|r| r.status == Status::XPass).count();
    let failed = rows.iter().filter(|r| r.status == Status::Fail).count();
    let errors = rows.iter().filter(|r| r.status == Status::Error).count();
    let skipped = rows.iter().filter(|r| r.status == Status::Skip).count();
    eprintln!(
        "-- PASS {passed}, XFAIL {xfailed}, XPASS {xpassed}, FAIL {failed}, ERROR {errors}, SKIP {skipped} ({} total) --",
        rows.len()
    );
    if options.strict && (failed > 0 || xpassed > 0 || errors > 0) {
        return Err(format!(
            "strict report found {failed} FAIL, {xpassed} XPASS, and {errors} ERROR result(s)"
        ));
    }
    Ok(())
}

fn run_diff(args: &[String]) -> Result<(), String> {
    let [before, after] = args else {
        return Err(USAGE.to_owned());
    };
    let load = |path: &String| -> Result<Vec<Row>, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        parse_tsv(&text)
    };
    print!("{}", render_diff(&diff(&load(before)?, &load(after)?)));
    Ok(())
}
