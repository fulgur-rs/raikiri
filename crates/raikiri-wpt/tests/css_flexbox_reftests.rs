//! Focused Flexbox gap/overflow visual checks.
//!
//! These pairs are measured at the project-wide exact 800x600 viewport. The
//! test is ignored by default because the sparse WPT checkout is fetched by
//! the WPT workflow.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Preserve the landed overflow passes and pin the newly measured mixed-gap
/// pairs without including the still-failing RTL or overflow-padding cases.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn flex_gap_and_overflow_pass_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-flexbox/gap-008-ltr.html",
        "css/css-flexbox/gap-009-ltr.html",
        "css/css-flexbox/gap-010-ltr.html",
        "css/css-flexbox/flexbox-overflow-horiz-002.html",
        "css/css-flexbox/flexbox-overflow-horiz-003.html",
        "css/css-flexbox/flexbox-overflow-horiz-004.html",
        "css/css-flexbox/flexbox-overflow-horiz-005.html",
        "css/css-flexbox/flexbox-overflow-vert-001.html",
        "css/css-flexbox/flexbox-overflow-vert-002.html",
        "css/css-flexbox/flexbox-overflow-vert-003.html",
        "css/css-flexbox/flexbox-overflow-vert-004.html",
        "css/css-flexbox/flexbox-overflow-vert-005.html",
        "css/css-flexbox/overflow-area-003.html",
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
