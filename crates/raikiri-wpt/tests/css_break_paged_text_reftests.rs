//! Focused CSS Break paged-text fragmentation checks.
//!
//! The measured set covers zero-height parallel-flow overflow and the
//! widows/orphans cases that are exact at the project-wide 800x600 viewport.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Verify the measured paged-text fragmentation pairs.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_break_paged_text_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-break/tall-line-in-short-fragmentainer-000.html",
        "css/css-break/widows-orphans-008.html",
        "css/css-break/widows-orphans-009.html",
        "css/css-break/widows-orphans-010.html",
        "css/css-break/widows-orphans-011.html",
        "css/css-break/widows-orphans-012.html",
        "css/css-break/widows-orphans-013.html",
        "css/css-break/widows-orphans-014.html",
        "css/css-break/widows-orphans-015.html",
        "css/css-break/widows-orphans-016.html",
        "css/css-break/widows-orphans-018.html",
    ];
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for relative in cases {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        assert_eq!(pairs.len(), 1, "expected one pair for {relative}");
        let result =
            run_pair(&pairs[0], config).unwrap_or_else(|error| panic!("run {relative}: {error}"));
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{relative}: {:?} ({} mismatched pixels)",
            result.outcome,
            result.mismatched_pixels
        );
    }
}
