//! Focused exact WPT coverage for CSS Text `line-break: anywhere`.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Run the pinned `line-break: anywhere` WPT slice against each local reference.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn line_break_anywhere_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // First-stage scope: exact static cases verified with WPT resources. The
    // wider survey has 12 unrelated abspos auto-width oracle failures, four
    // inline-run cases that need shared paragraph layout, and one case blocked
    // by U+2011 font fallback. `anywhere-003` also needs inline JavaScript, which
    // this static runner does not execute. Keep these cases visible in the linked
    // issue; do not mask or baseline them here.
    let candidates = [
        "css/css-text/line-break/line-break-anywhere-002.html",
        "css/css-text/line-break/line-break-anywhere-004.html",
        "css/css-text/line-break/line-break-anywhere-005.html",
        "css/css-text/line-break/line-break-anywhere-006.html",
        "css/css-text/line-break/line-break-anywhere-007.html",
        "css/css-text/line-break/line-break-anywhere-008.html",
        "css/css-text/line-break/line-break-anywhere-009.html",
        "css/css-text/line-break/line-break-anywhere-010.html",
        "css/css-text/line-break/line-break-anywhere-011.html",
        "css/css-text/line-break/line-break-anywhere-012.html",
        "css/css-text/line-break/line-break-anywhere-013.html",
        "css/css-text/line-break/line-break-anywhere-014.html",
        "css/css-text/line-break/line-break-anywhere-015.html",
        "css/css-text/line-break/line-break-anywhere-016.html",
        "css/css-text/line-break/line-break-anywhere-and-white-space-002.html",
        "css/css-text/line-break/line-break-anywhere-and-white-space-005.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-001.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-002.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-003.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-004.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-006.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-010.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-012.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-015.html",
    ];
    assert_eq!(candidates.len(), 24);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    let mut failures = Vec::new();
    for relative in candidates {
        let test = root.join(&relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        if pairs.len() != 1 {
            failures.push(format!(
                "{relative}: expected 1 match pair, found {}",
                pairs.len()
            ));
            continue;
        }
        match run_pair_with_images(&pairs[0], config) {
            Ok(result) if matches!(&result.outcome, TestOutcome::Pass) => {}
            Ok(result) => failures.push(format!(
                "{relative}: outcome={:?}, mismatched_pixels={}",
                result.outcome, result.mismatched_pixels
            )),
            Err(error) => failures.push(format!("{relative}: run error: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "line-break:anywhere WPT failures at exact 800x600:\n{}",
        failures.join("\n")
    );
}
