//! Focused paged-media @layer visual checks.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::TestOutcome;

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn page_layers_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    for relative in [
        "css/css-page/layers-001-print.html",
        "css/css-page/layers-002-print.html",
        "css/css-page/layers-003-print.html",
        "css/css-page/layers-004-print.html",
    ] {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root)).unwrap();
        assert_eq!(pairs.len(), 1, "expected one pair for {relative}");
        let result = run_pair(&pairs[0], config).unwrap();
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{relative}: {:?} ({} mismatched pixels)",
            result.outcome,
            result.mismatched_pixels
        );
    }
}
