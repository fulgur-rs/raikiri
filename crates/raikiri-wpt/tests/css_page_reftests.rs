//! Focused CSS Page visual checks at the exact project viewport.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Named pages and page-margin geometry match their references exactly.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn named_page_and_margin_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for name in [
        "page-name-000-print.html",
        "page-name-001-print.html",
        "page-name-002-print.html",
        "page-margin-001-print.html",
        "page-margin-002-print.html",
        "page-margin-003-print.html",
        "page-margin-004-print.html",
        "page-margin-005-print.html",
        "page-margin-006-print.html",
    ] {
        run_exact_pair(&root, "css/css-page", name, config);
    }
}

/// A named page box containing only a `display:none` child still opens its
/// explicit empty page boundary.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn page_name_display_none_child_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    run_exact_pair(
        &root,
        "css/css-page",
        "page-name-display-none-child-print.html",
        config,
    );
}

/// Page counters in margin boxes match their references exactly.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn page_counter_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for name in [
        "content-004-print.html",
        "content-005-print.html",
        "content-008-print.html",
        "content-009-print.html",
    ] {
        run_exact_pair(&root, "css/css-page/margin-boxes", name, config);
    }
}

/// Spread-pseudo side margins constrain the page content width exactly.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn page_left_right_spread_pseudo_pair_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    run_exact_pair(
        &root,
        "css/css-page",
        "page-left-right-001-print.html",
        config,
    );
}

/// Named-page propagation through a column flex container is pixel exact.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn page_name_flex_003_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    run_exact_pair(
        &root,
        "css/css-page",
        "page-name-flex-003-print.html",
        config,
    );
}

/// Nested named-page propagation through a column flex container is pixel exact.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn page_name_flex_004_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    run_exact_pair(
        &root,
        "css/css-page",
        "page-name-flex-004-print.html",
        config,
    );
}

fn run_exact_pair(root: &std::path::Path, dir: &str, name: &str, config: ReftestConfig) {
    let test = root.join(dir).join(name);
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(root))
        .unwrap_or_else(|error| panic!("discover {}: {error}", test.display()));
    assert_eq!(pairs.len(), 1, "expected one pair for {}", test.display());
    let result = run_pair(&pairs[0], config)
        .unwrap_or_else(|error| panic!("run {}: {error}", test.display()));
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{}: {:?} ({} mismatched pixels)",
        test.display(),
        result.outcome,
        result.mismatched_pixels
    );
}

/// Named-page declarations on out-of-flow boxes do not create page transitions.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn out_of_flow_named_page_boxes_match_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for name in [
        "page-name-abspos-001-print.html",
        "page-name-abspos-002-print.html",
    ] {
        run_exact_pair(&root, "css/css-page", name, config);
    }
}
