//! Focused CSS Break flex/float visual checks.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Verify float height propagation through a fragmented flex item.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn flex_fragmented_with_float_descendant_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-break/flexbox/flex-fragmented-with-float-descendant-001.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .expect("discover flex float pair");
    assert_eq!(pairs.len(), 1, "expected one pair for flex float case");

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    let result = run_pair(&pairs[0], config).expect("run flex float pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
}
