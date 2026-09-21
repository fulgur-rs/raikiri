//! Focused CSS Break Grid visual checks.
//!
//! These pairs are exact at the project-wide 800x600 viewport. They cover
//! grid-item fragmentation, overflow, and monolithic overflow behavior.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Verify the measured CSS Break Grid reftest pairs.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_break_grid_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-break/grid/grid-container-fragmentation-001.html",
        "css/css-break/grid/grid-item-fragmentation-001.html",
        "css/css-break/grid/grid-item-fragmentation-002.html",
        "css/css-break/grid/grid-item-fragmentation-003.html",
        "css/css-break/grid/grid-item-fragmentation-014.html",
        "css/css-break/grid/grid-item-fragmentation-016.html",
        "css/css-break/grid/grid-item-fragmentation-017.html",
        "css/css-break/grid/grid-item-fragmentation-019.html",
        "css/css-break/grid/grid-item-fragmentation-020.html",
        "css/css-break/grid/grid-item-fragmentation-030.html",
        "css/css-break/grid/grid-item-fragmentation-031.html",
        "css/css-break/grid/grid-item-fragmentation-035.html",
        "css/css-break/grid/grid-item-fragmentation-036.html",
        "css/css-break/grid/grid-item-fragmentation-037.html",
        "css/css-break/grid/grid-item-fragmentation-038.html",
        "css/css-break/grid/grid-item-fragmentation-043.html",
        "css/css-break/grid/grid-item-fragmentation-044.html",
        "css/css-break/grid/grid-item-fragmentation-045.html",
        "css/css-break/grid/grid-item-fragmentation-046.html",
        "css/css-break/grid/grid-item-fragmentation-047.html",
        "css/css-break/grid/grid-item-fragmentation-048.html",
        "css/css-break/grid/grid-item-infinite-expansion.html",
        "css/css-break/grid/monolithic-overflow-005.html",
        "css/css-break/grid/monolithic-overflow-006.html",
        "css/css-break/grid/monolithic-overflow-009.html",
        "css/css-break/grid/subgrid/subgrid-container-fragmentation-001.html",
        "css/css-break/grid/subgrid/subgrid-container-fragmentation-002.html",
        "css/css-break/grid/subgrid/subgrid-item-fragmentation-001.html",
        "css/css-break/grid/subgrid/subgrid-item-fragmentation-002.html",
        "css/css-break/grid/subgrid/subgrid-item-fragmentation-003.html",
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
