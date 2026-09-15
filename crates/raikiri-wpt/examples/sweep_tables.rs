//! Scratch sweep (untracked): all css-tables pairs EXACT 800x600.
use raikiri_wpt::reftest::{ReftestConfig, discover_all_pairs_with_docroot};
use raikiri_wpt::runner::Tolerance;
use std::path::PathBuf;
fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let wpt_root = manifest.join("../../target/wpt");
    let tables = wpt_root.join("css/css-tables");
    let pairs = discover_all_pairs_with_docroot(&tables, &wpt_root);
    eprintln!("discovered pairs: {}", pairs.len());
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let mut pass = 0usize;
    let mut fail = 0usize;
    for pair in &pairs {
        let test_id = pair
            .test
            .strip_prefix(&wpt_root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| pair.test.display().to_string());
        match raikiri_wpt::reftest::run_pair(pair, config) {
            Ok(r) => match r.outcome {
                raikiri_wpt::runner::TestOutcome::Pass => {
                    pass += 1;
                    println!("PASS css/{} kind={:?}", test_id, pair.kind);
                }
                raikiri_wpt::runner::TestOutcome::Fail(_) => {
                    fail += 1;
                    println!(
                        "FAIL css/{} kind={:?} mismatched={}",
                        test_id, pair.kind, r.mismatched_pixels
                    );
                }
                other => {
                    println!("OTHER css/{} :: {:?}", test_id, other);
                }
            },
            Err(e) => {
                println!("ERROR css/{} :: {}", test_id, e);
            }
        }
    }
    eprintln!("SUMMARY pass={} fail={} total={}", pass, fail, pairs.len());
}
