//! Pixel-exact relative block positioning regressions.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, ReftestTolerance, discover_pairs_for_file_with_wpt_root, run_pair_with_images,
};
use raikiri_wpt::runner::TestOutcome;

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn relative_block_insets_are_applied_once_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = ReftestTolerance::EXACT;
    config.require_inline_fonts = true;
    for id in [
        "css/CSS2/positioning/position-relative-004.xht",
        "css/CSS2/positioning/position-relative-005.xht",
        "css/CSS2/positioning/position-relative-006.xht",
        "css/CSS2/positioning/position-relative-007.xht",
        "css/CSS2/positioning/position-relative-009.xht",
        "css/CSS2/positioning/position-relative-010.xht",
        "css/CSS2/positioning/position-relative-013.xht",
        "css/CSS2/positioning/position-relative-014.xht",
        "css/CSS2/positioning/position-relative-015.xht",
        "css/CSS2/positioning/position-relative-016.xht",
        "css/CSS2/positioning/position-relative-018.xht",
        "css/CSS2/positioning/position-relative-019.xht",
        "css/CSS2/positioning/position-relative-037.xht",
        "css/CSS2/positioning/position-relative-038.xht",
    ] {
        let pairs = discover_pairs_for_file_with_wpt_root(&root.join(id), Some(&root))
            .expect("discover relative positioning pair");
        assert_eq!(pairs.len(), 1, "{id}");
        let result =
            run_pair_with_images(&pairs[0], config).expect("run relative positioning pair");
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{id}: {:?}",
            result.outcome
        );
        assert_eq!(result.mismatched_pixels, 0, "{id}");
    }
}
