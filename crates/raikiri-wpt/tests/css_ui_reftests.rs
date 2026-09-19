//! Focused CSS Basic User Interface visual checks.
//!
//! These tests run selected outline WPT pairs at the project-wide exact
//! 800x600 viewport. They are ignored by default because the sparse WPT
//! checkout is fetched separately by the WPT workflow.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// A solid outline with a positive offset matches the CSS UI reference pair.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn outline_offset_basic_pair_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-ui/outline-offset.html");
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
