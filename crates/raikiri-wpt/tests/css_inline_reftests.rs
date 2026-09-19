//! Focused WPT reftests for the minimal CSS Inline foundation.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Run the direct `vertical-align: top|bottom` padding pair at the pinned
/// 800x600 viewport. Fetch the sparse WPT checkout first.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn vertical_align_top_bottom_padding_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/CSS2/linebox/vertical-align-top-bottom-padding.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(&result.outcome, TestOutcome::Pass),
        "outcome: {:?}",
        result.outcome
    );
}
