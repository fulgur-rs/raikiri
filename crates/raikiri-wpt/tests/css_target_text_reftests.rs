//! Focused CSS Generated Content target-text checks.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::TestOutcome;

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn target_text_exact_800x600_subset() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let tests = [
        "css/css-pseudo/target-text-008.html",
        "css/css-pseudo/target-text-009.html",
        "css/css-pseudo/target-text-011.html",
        "css/css-pseudo/target-text-dynamic-003.html",
    ];
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    for relative in tests {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root)).unwrap();
        assert_eq!(pairs.len(), 1, "{relative}: expected one reference pair");
        let result = run_pair(&pairs[0], config).unwrap();
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{relative}: {:?} ({} mismatched pixels)",
            result.outcome,
            result.mismatched_pixels
        );
    }
}
