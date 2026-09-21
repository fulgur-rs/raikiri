//! Focused ruby fragmentation visual checks.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::TestOutcome;

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn ruby_fragmentation_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-break/ruby-003.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root)).unwrap();
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    let result = run_pair(&pairs[0], config).unwrap();
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{test:?}: {:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
}
