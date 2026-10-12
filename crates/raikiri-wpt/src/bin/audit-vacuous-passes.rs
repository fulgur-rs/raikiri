//! Audit passing reftests for vacuous PASS before promoting them.
//!
//! ```text
//! cargo run --release --locked -p raikiri-wpt --bin audit-vacuous-passes -- \
//!     --wpt-root target/wpt --ids candidates.txt --output audit.tsv --jobs 4
//! ```
//!
//! Each id is run once; a passing reftest is then probed as described in
//! [`raikiri_wpt::vacuity`]. The report has one `id<TAB>VERDICT<TAB>detail`
//! line per id. Ids marked `KEEP` are candidates for the pass baseline; the
//! `VACUOUS-*` verdicts name why a PASS does not show the feature working.

use raikiri_wpt::baseline_report::{Row, Status, parse_baseline_ids, run_ids_with};
use raikiri_wpt::vacuity::{Verdict, audit_id};
use std::path::PathBuf;

const USAGE: &str =
    "usage: audit-vacuous-passes [--wpt-root DIR] --ids FILE [--output FILE] [--jobs N]";

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut wpt_root = PathBuf::from("target/wpt");
    let mut ids_path = None;
    let mut output = None;
    let mut jobs = 1usize;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = || {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--wpt-root" => wpt_root = PathBuf::from(value()?),
            "--ids" => ids_path = Some(PathBuf::from(value()?)),
            "--output" => output = Some(PathBuf::from(value()?)),
            "--jobs" => {
                jobs = value()?
                    .parse()
                    .ok()
                    .filter(|jobs| *jobs > 0)
                    .ok_or("--jobs expects a positive number")?;
            }
            other => return Err(format!("unrecognized argument: {other}\n{USAGE}")),
        }
        index += 2;
    }
    let ids_path = ids_path.ok_or(USAGE)?;
    let text = std::fs::read_to_string(&ids_path)
        .map_err(|e| format!("read {}: {e}", ids_path.display()))?;
    let ids = parse_baseline_ids(&text);
    raikiri_wpt::reftest::check_inline_formatting_fonts()
        .map_err(|e| format!("inline engine fonts: {e}"))?;
    eprintln!("auditing {} tests with {jobs} job(s)", ids.len());
    // Reuse the baseline runner's worker pool; the row status carries
    // whether the verdict keeps the id.
    let rows = run_ids_with(
        &ids,
        jobs,
        |id| {
            let verdict =
                std::panic::catch_unwind(|| audit_id(&wpt_root, id)).unwrap_or(Verdict::NotPass {
                    status: Status::Error,
                    detail: "panic during audit".to_owned(),
                });
            Row {
                id: id.to_owned(),
                status: if matches!(verdict, Verdict::Kept { .. }) {
                    Status::Pass
                } else {
                    Status::Skip
                },
                detail: format!("{}\t{}", verdict.label(), verdict.detail()),
            }
        },
        &|row: &Row| eprintln!("{}\t{}", row.id, row.detail),
    );
    let mut report = String::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for row in &rows {
        let detail = row.detail.replace('\n', " ");
        report.push_str(&format!("{}\t{detail}\n", row.id));
        let label = detail.split('\t').next().unwrap_or_default().to_owned();
        *counts.entry(label).or_default() += 1;
    }
    match &output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create {}: {e}", parent.display()))?;
            }
            std::fs::write(path, &report).map_err(|e| format!("write {}: {e}", path.display()))?;
        }
        None => print!("{report}"),
    }
    let summary = counts
        .iter()
        .map(|(label, count)| format!("{label} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!("-- {summary} ({} total) --", rows.len());
    Ok(())
}
