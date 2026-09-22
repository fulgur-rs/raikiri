//! Focused CSS Break Flexbox parity checks.
//!
//! These pairs are exact at the project-wide 800x600 viewport. They cover
//! single-line column and row items plus one multi-line row case that the
//! current pagination implementation can render without approximation.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Verify the Fulgur v0.40.0 PASS CSS Break Flexbox print slice.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_break_flexbox_print_fragmentation_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // Keep this parity slice limited to Fulgur v0.40.0 PASS cases.
    // The deferred 068b/069d cases remain outside the exact parity gate.
    let cases = [
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-080-print.html",
        "css/css-break/flexbox/single-line-column-flex-fragmentation-065-print.html",
        "css/css-break/flexbox/single-line-column-flex-fragmentation-066-print.html",
        "css/css-break/flexbox/single-line-column-flex-fragmentation-068a-print.html",
        "css/css-break/flexbox/single-line-column-flex-fragmentation-069a-print.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-045-print.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-046-print.html",
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
