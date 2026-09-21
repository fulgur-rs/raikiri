//! Focused CSS Fonts visual checks.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Check a variable-font oblique request that must not synthesize outside its capabilities.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn synthetic_oblique_out_of_capabilities_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-fonts/synthetic-oblique-out-of-capabilities-range.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .expect("discover CSS Fonts WPT pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run CSS Fonts WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{}: {:?} ({} mismatched pixels)",
        test.display(),
        result.outcome,
        result.mismatched_pixels
    );
}
