//! Focused Flex/Grid absolute-position static-position checks.
//!
//! These pairs were measured at the project-wide exact 800x600 viewport.
//! The test is ignored by default because the sparse WPT checkout is fetched
//! by the WPT workflow.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Preserve verified Flex passes and cover Grid static-position alignment.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn flex_and_grid_abspos_static_position_pass_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-flexbox/abspos/position-absolute-006.html",
        "css/css-flexbox/abspos/position-absolute-007.html",
        "css/css-flexbox/abspos/position-absolute-008.html",
        "css/css-flexbox/abspos/position-absolute-009.html",
        "css/css-flexbox/abspos/position-absolute-010.html",
        "css/css-flexbox/abspos/position-absolute-011.html",
        "css/css-flexbox/abspos/position-absolute-containing-block-001.html",
        "css/css-grid/abspos/grid-abspos-staticpos-align-items-center.html",
        "css/css-grid/abspos/grid-abspos-staticpos-align-items-end-large-border-padding.html",
        "css/css-grid/abspos/grid-abspos-staticpos-align-self-center.html",
        "css/css-grid/abspos/grid-abspos-staticpos-align-self-flex-end-large-border-padding.html",
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
