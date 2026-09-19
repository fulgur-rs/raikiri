//! Focused CSS Multi-column visual checks.
//!
//! These pairs were measured at the project-wide exact 800x600 viewport.
//! The test is ignored by default because the sparse WPT checkout is fetched
//! by the WPT workflow.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Keep verified Multi-column passes covered across the first precision groups.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_multicol_pass_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        // Basic column layout.
        "css/css-multicol/multicol-basic-005.xht",
        "css/css-multicol/multicol-basic-006.xht",
        "css/css-multicol/multicol-basic-007.xht",
        "css/css-multicol/multicol-basic-008.xht",
        // Balancing and break avoidance.
        "css/css-multicol/balance-break-avoidance-000.html",
        "css/css-multicol/balance-break-avoidance-001.html",
        // Spanning, vertical writing, and fragmentainer sizing.
        "css/css-multicol/multicol-span-all-004.html",
        "css/css-multicol/multicol-under-vertical-rl-scroll.html",
        "css/css-multicol/change-fragmentainer-size-000.html",
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
