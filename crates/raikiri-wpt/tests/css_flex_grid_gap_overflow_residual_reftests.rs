//! Focused Flexbox gap/overflow residual checks.
//!
//! These pairs were remeasured at the project-wide exact 800x600 viewport.
//! The test is ignored by default because the sparse WPT checkout is fetched
//! by the WPT workflow.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Preserve the four exact gap-002 PASS variants while residual failures stay unpinned.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn flex_gap_002_pass_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-flexbox/gap-002-ltr.html",
        "css/css-flexbox/gap-002-rtl.html",
        "css/css-flexbox/gap-002-lr.html",
        "css/css-flexbox/gap-002-rl.html",
    ];
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for relative in cases {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        assert_eq!(pairs.len(), 1, "expected one pair for {relative}");
        let result =
            run_pair(&pairs[0], config).unwrap_or_else(|error| panic!("run {relative}: {error}"));
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{relative}: {:?} ({} mismatched pixels)",
            result.outcome,
            result.mismatched_pixels
        );
    }
}
