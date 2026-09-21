//! Focused CSS Overflow visual checks at the exact project viewport.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Basic clipping, canvas overflow, and body propagation cases match exactly.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn overflow_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for name in [
        "clip-001.html",
        "overflow-canvas.html",
        "overflow-body-propagation-001.html",
        "overflow-body-propagation-002.html",
    ] {
        let test = root.join("css/css-overflow").join(name);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {}: {error}", test.display()));
        assert_eq!(pairs.len(), 1, "expected one pair for {}", test.display());
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
}
