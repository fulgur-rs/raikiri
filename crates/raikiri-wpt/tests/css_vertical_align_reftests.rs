//! Focused vertical-align visual checks.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, ReftestTolerance, discover_pairs_for_file_with_wpt_root, run_pair,
    run_pair_with_images,
};
use raikiri_wpt::runner::TestOutcome;

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn vertical_align_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    for relative in [
        "css/CSS2/linebox/vertical-align-top-bottom-padding.html",
        "css/css-values/calc-vertical-align-1.html",
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
        assert_eq!(
            result.mismatched_pixels, 0,
            "{relative}: expected zero mismatched pixels at exact tolerance"
        );
    }
}

/// The CSS2 .xht case verifies a positive length shift against positioned
/// image references. Keep the explicit pair because the survey does not
/// auto-discover .xht files.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn vertical_align_length_96px_wpt_pair_matches_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/CSS2/linebox/vertical-align-007.xht");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .expect("discover vertical-align length WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = ReftestTolerance::TIER2;
    let result =
        run_pair_with_images(&pairs[0], config).expect("run vertical-align length WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
    // This WPT scales a 15×15 PNG to 20×20 at a fractional origin. Its four
    // edge rows differ from hinted Ahem glyphs; cap that known fringe at 80.
    assert!(
        result.mismatched_pixels <= 80,
        "image/Ahem edge-AA fringe exceeded 80 pixels: {}",
        result.mismatched_pixels
    );
}
