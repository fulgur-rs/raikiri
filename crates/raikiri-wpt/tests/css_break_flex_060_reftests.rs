//! Focused CSS Break Flexbox pagination regression coverage.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Keep the trailing column-flex item on the same fragmentainer as its peers.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn single_line_column_flex_060_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let relative = "css/css-break/flexbox/single-line-column-flex-fragmentation-060-print.html";
    let test = root.join(relative);
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover flex 060 pair");
    assert_eq!(pairs.len(), 1, "expected one pair for flex 060");

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run flex 060 pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{relative}: {:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
}
