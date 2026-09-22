//! Focused CSS Tables visual checks.
//!
//! These pairs were measured at the project-wide exact 800x600 viewport.
//! The test is ignored by default because the sparse WPT checkout is fetched
//! by the WPT workflow.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Verify the first measured auto-layout colspan distribution pair.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_tables_colspan_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = ["css/css-tables/colspan-004.html"];
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

/// Verify the vertical-writing-mode table baseline pair after logical-axis mapping.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_tables_vertical_baseline_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let relative = "css/css-tables/baseline-vertical.html";
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

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

/// Verify measured caption, anonymous-table, and percentage-height pairs.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_tables_caption_percentage_and_anonymous_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-tables/caption-relative-positioning.html",
        "css/css-tables/anonymous-table-ws-001.html",
        "css/css-tables/html-display-table.html",
        "css/css-tables/percent-height-replaced-in-percent-cell.tentative.html",
        "css/css-tables/height-distribution/percentage-sizing-of-table-cell-children-002.html",
        "css/css-tables/height-distribution/percentage-sizing-of-table-cell-children-003.html",
        "css/css-tables/height-distribution/percentage-sizing-of-table-cell-children-004.html",
        "css/css-tables/height-distribution/percentage-sizing-of-table-cell-replaced-children-001.html",
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

/// Verify the absolute-positioned table available-size pairs.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_tables_absolute_positioned_table_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-tables/absolute-tables-008.tentative.html",
        "css/css-tables/absolute-tables-011.tentative.html",
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

/// Verify measured border-collapse and table-border-painting PASS pairs.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_tables_border_collapse_and_paint_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-tables/border-collapse-double-border.html",
        "css/css-tables/border-collapse-dynamic-oof.html",
        "css/css-tables/border-collapse-empty-cell.html",
        "css/css-tables/border-collapse-rowspan-cell.html",
        "css/css-tables/border-conflict-resolution.html",
        "css/css-tables/collapsed-border-paint-phase-001.html",
        "css/css-tables/collapsed-border-partial-invalidation-003.html",
        "css/css-tables/collapsed-border-positioned-tr-td.html",
        "css/css-tables/collapsed-border-sideways-rl-rtl-overflow.html",
        "css/css-tables/collapsed-border-vertical-lr-rtl-overflow.html",
        "css/css-tables/collapsed-border-vertical-rtl-overflow.html",
        "css/css-tables/out-of-order-elements-collapsed-border.html",
        "css/css-tables/paint/col-change-span-bg-invalidation-002.html",
        "css/css-tables/paint/col-paint-htb-rtl.html",
        "css/css-tables/paint/table-border-paint-caption-change.html",
        "css/css-tables/rowspan-cell-border-after-color.html",
        "css/css-tables/table-has-box-sizing-border-box-001.html",
        "css/css-tables/visibility-collapse-rowspan-005.html",
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
