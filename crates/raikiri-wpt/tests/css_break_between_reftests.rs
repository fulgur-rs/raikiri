//! Pinned CSS Break between-box avoidance at 800 × 600 with bundled WPT fonts.

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};
use std::path::PathBuf;

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn between_box_avoidance_references_are_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.require_inline_fonts = true;
    config.tolerance = Tolerance::EXACT;
    for index in 0..15 {
        let id = format!("css/css-break/break-between-avoid-{index:03}.html");
        let pairs = discover_pairs_for_file_with_wpt_root(&root.join(&id), Some(&root)).unwrap();
        assert_eq!(pairs.len(), 1, "{id}");
        let result = run_pair_with_images(&pairs[0], config).unwrap();
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{id}: {:?}",
            result.outcome
        );
        assert_eq!(result.mismatched_pixels, 0, "{id}");
    }
}
