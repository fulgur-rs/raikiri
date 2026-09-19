//! Focused CSS Color visual checks.
//!
//! These tests run selected opacity WPT pairs at the project-wide 800x600
//! exact viewport. They are ignored by default because the sparse WPT checkout
//! is fetched separately by the WPT workflow.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Nested content under opacity must composite as one element group at 800x600.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn opacity_offscreen_group_pair_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-color/t32-opacity-offscreen-b.xht");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .unwrap_or_else(|error| panic!("discover {}: {error}", test.display()));
    assert_eq!(pairs.len(), 1, "expected one pair for {}", test.display());
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
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
