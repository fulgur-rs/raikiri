//! Additional exact CSS Break Flexbox reftest coverage.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Verify additional CSS Break Flexbox pairs measured exact at 800x600.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn additional_css_break_flexbox_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-break/flexbox/flex-container-fragmentation-008.html",
        "css/css-break/flexbox/flex-container-fragmentation-009.html",
        "css/css-break/flexbox/flex-item-content-overflow-001a.html",
        "css/css-break/flexbox/flex-item-content-overflow-001b.html",
        "css/css-break/flexbox/flex-item-content-overflow-002a.html",
        "css/css-break/flexbox/flex-item-content-overflow-002b.html",
        "css/css-break/flexbox/flex-item-content-overflow-003.html",
        "css/css-break/flexbox/increase-fragmentainer-size-flex-item-trailing-margin.html",
        "css/css-break/flexbox/monolithic-overflow-001.tentative.html",
        "css/css-break/flexbox/monolithic-overflow-002.tentative.html",
        "css/css-break/flexbox/multi-line-column-flex-fragmentation-012.html",
        "css/css-break/flexbox/multi-line-column-flex-fragmentation-041.html",
        "css/css-break/flexbox/multi-line-column-flex-fragmentation-050.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-009.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-018.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-022.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-023.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-024.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-037.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-038.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-045.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-061.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-081a-print.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-081b-print.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-083a.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-083b.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-083c.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-083d.html",
        "css/css-break/flexbox/multi-line-row-flex-fragmentation-091.html",
        "css/css-break/flexbox/single-line-column-flex-fragmentation-007.html",
        "css/css-break/flexbox/single-line-column-flex-fragmentation-008.html",
        "css/css-break/flexbox/single-line-column-flex-fragmentation-009.html",
        "css/css-break/flexbox/single-line-column-flex-fragmentation-040.html",
        "css/css-break/flexbox/single-line-column-flex-fragmentation-056.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-009.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-021.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-038.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-049.html",
        "css/css-break/flexbox/single-line-row-flex-fragmentation-050.html",
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
