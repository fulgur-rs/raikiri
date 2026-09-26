//! Focused css/printing fragmented-inline-block checks at the exact project viewport.
//!
//! Theme: inline-block fragmentation across printed pages (001 expects a
//! multi-page diff vs blank via `mismatch`; 002 matches its tall-block
//! reference via `match`). Both pairs PASS at 800x600 EXACT by asserting
//! [`TestOutcome::Pass`] rather than zero pixels, so the expected 001 diff
//! counts as PASS.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Fragmented inline-blocks paginate across print pages (001 mismatch-PASS,
/// 002 match-PASS) at 800x600 EXACT.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn fragmented_inline_block_pairs_pass_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for name in [
        "fragmented-inline-block-001-print.html",
        "fragmented-inline-block-002-print.html",
    ] {
        run_pass_pair(&root, "css/printing", name, config);
    }
}

fn run_pass_pair(root: &std::path::Path, dir: &str, name: &str, config: ReftestConfig) {
    let test = root.join(dir).join(name);
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(root))
        .unwrap_or_else(|error| panic!("discover {}: {error}", test.display()));
    assert_eq!(pairs.len(), 1, "expected one pair for {}", test.display());
    let result = run_pair(&pairs[0], config)
        .unwrap_or_else(|error| panic!("run {}: {error}", test.display()));
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{}: {:?} ({} mismatched pixels)",
        test.display(),
        result.outcome,
        result.mismatched_pixels
    );
}
