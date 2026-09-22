//! Additional exact CSS Break Flexbox parity coverage.
//!
//! This slice contains only cases that also pass in Fulgur v0.40.0.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Verify the Fulgur v0.40.0 PASS CSS Break Flexbox slice at 800x600.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn additional_css_break_flexbox_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // Keep this slice limited to cases that Fulgur v0.40.0 also passes.
    // Raikiri/Fulgur both-FAIL cases stay deferred until separately justified.
    let cases = [
        "css/css-break/flexbox/flex-item-content-overflow-001a.html",
        "css/css-break/flexbox/flex-item-content-overflow-001b.html",
        "css/css-break/flexbox/flex-item-content-overflow-002a.html",
        "css/css-break/flexbox/flex-item-content-overflow-002b.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-021.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-049.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-050.html",
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
