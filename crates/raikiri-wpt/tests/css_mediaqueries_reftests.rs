//! Pinned upstream media-query coverage at 800 × 600 with bundled WPT fonts.

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};
use std::path::PathBuf;

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn pinned_media_query_reftests_are_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.require_inline_fonts = true;
    config.tolerance = Tolerance::EXACT;
    for id in [
        "css/mediaqueries/aspect-ratio-002.html",
        "css/mediaqueries/device-aspect-ratio-004.html",
        "css/mediaqueries/device-aspect-ratio-006.html",
        "css/mediaqueries/mq-calc-006.html",
        "css/mediaqueries/mq-calc-sign-function-006.html",
        "css/mediaqueries/mq-deprecated-001.html",
        "css/mediaqueries/mq-gamut-003.html",
        "css/mediaqueries/mq-gamut-005.html",
        "css/mediaqueries/mq-invalid-media-type-001.html",
        "css/mediaqueries/mq-invalid-media-type-002.html",
        "css/mediaqueries/mq-invalid-media-type-003.html",
        "css/mediaqueries/mq-invalid-media-type-004.html",
        "css/mediaqueries/mq-invalid-media-type-layer-001.html",
        "css/mediaqueries/mq-range-001.html",
        "css/mediaqueries/prefers-color-scheme-svg-image-normal-with-meta-dark.html",
        "css/mediaqueries/prefers-color-scheme-svg-image-normal-with-meta-light.html",
        "css/mediaqueries/prefers-color-scheme-svg-image-normal.html",
        "css/mediaqueries/prefers-color-scheme-svg-image.html",
        "css/mediaqueries/relative-units-001.html",
        "css/mediaqueries/relative-units-002.html",
        "css/mediaqueries/relative-units-003.html",
        "css/mediaqueries/relative-units-004.html",
    ] {
        let pairs = discover_pairs_for_file_with_wpt_root(&root.join(id), Some(&root)).unwrap();
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
