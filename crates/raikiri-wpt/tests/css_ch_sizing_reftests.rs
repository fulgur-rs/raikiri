//! Exact CSS sizing with font-derived ch constraints.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_overflow_inherited_ahem_max_width_ch_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let path = root.join("css/css-overflow/text-overflow-ellipsis-002.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&path, Some(&root)).expect("discover pairs");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    config.require_inline_fonts = true;
    let result = run_pair(&pairs[0], config).expect("render inherited Ahem max-width");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{:?}: {} mismatched pixels",
        result.outcome,
        result.mismatched_pixels
    );
}
