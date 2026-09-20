//! Focused exact WPT reftests for CSS Text residual coverage.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn overflow_wrap_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/overflow-wrap/overflow-wrap-cluster-001.html",
        "css/css-text/overflow-wrap/overflow-wrap-cluster-002.html",
        "css/css-text/overflow-wrap/overflow-wrap-min-content-size-006.html",
    ];

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for relative in candidates {
        let test = root.join(relative);
        let pairs =
            discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
        assert_eq!(pairs.len(), 1, "{relative}");
        let result = run_pair(&pairs[0], config).expect("run WPT pair");
        assert!(
            matches!(&result.outcome, TestOutcome::Pass),
            "{relative}: outcome={:?}, mismatches={}",
            result.outcome,
            result.mismatched_pixels
        );
    }
}
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn letter_spacing_ligatures_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/letter-spacing/letter-spacing-ligatures-004.html");
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
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn writing_system_font_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/writing-system/writing-system-font-001.html");
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
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}
