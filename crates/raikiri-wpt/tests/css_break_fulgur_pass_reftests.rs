//! CSS Break cases that are exact in both Raikiri and Fulgur v0.40.0.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Verify the current Fulgur v0.40.0 PASS slice at 800x600.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_break_fulgur_pass_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-break/abspos-in-clipped-overflow-print.html",
        "css/css-break/block-max-height-001.html",
        "css/css-break/block-max-height-001b.html",
        "css/css-break/block-max-height-002.html",
        "css/css-break/block-max-height-002b.html",
        "css/css-break/block-max-height-003.html",
        "css/css-break/block-max-height-003b.html",
        "css/css-break/block-max-height-004.html",
        "css/css-break/block-min-height-001.html",
        "css/css-break/block-min-height-001b.html",
        "css/css-break/border-image-000.html",
        "css/css-break/overflowed-abs-pos-with-percentage-height-print.html",
        "css/css-break/widows-block-in-inline-001.html",
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
