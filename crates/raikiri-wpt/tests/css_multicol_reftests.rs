//! Focused CSS Multi-column visual checks.
//!
//! These pairs were measured at the project-wide exact 800x600 viewport.
//! The test is ignored by default because the sparse WPT checkout is fetched
//! by the WPT workflow.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Keep verified Multi-column passes covered across the first precision groups.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_multicol_pass_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        // Basic column layout.
        "css/css-multicol/multicol-basic-005.xht",
        "css/css-multicol/multicol-basic-006.xht",
        "css/css-multicol/multicol-basic-007.xht",
        "css/css-multicol/multicol-basic-008.xht",
        // Balancing and break avoidance.
        "css/css-multicol/balance-break-avoidance-000.html",
        "css/css-multicol/balance-break-avoidance-001.html",
        "css/css-multicol/balance-break-avoidance-002.html",
        // Spanning, vertical writing, and fragmentainer sizing.
        "css/css-multicol/multicol-span-all-004.html",
        "css/css-multicol/multicol-under-vertical-rl-scroll.html",
        "css/css-multicol/change-fragmentainer-size-000.html",
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

/// Keep the foundational columns/count/width implementation covered with
/// resource-enabled rendering (Ahem and the bundled image reference).
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_multicol_foundation_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-multicol/multicol-basic-001.html",
        "css/css-multicol/multicol-basic-002.html",
        "css/css-multicol/multicol-basic-003.html",
        "css/css-multicol/multicol-basic-004.html",
        "css/css-multicol/multicol-count-001.xht",
        "css/css-multicol/multicol-count-002.xht",
        "css/css-multicol/column-count-used-001.html",
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
        let result = run_pair_with_images(&pairs[0], config)
            .unwrap_or_else(|error| panic!("run {relative}: {error}"));
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{relative}: {:?} ({} mismatched pixels)",
            result.outcome,
            result.mismatched_pixels
        );
    }
}

/// Verify the recursive block-flow boundary against selected nested WPT pairs.
///
/// These are intentionally ignored like the other WPT checks because they
/// require the sparse checkout. The cases cover an empty nested flow, definite
/// `column-fill:auto` block flow, the text-bearing nested probe, and the
/// already-passing percentage-gap/positioning regression.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_multicol_nested_block_flow_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-multicol/multicol-nested-025.html",
        "css/css-multicol/multicol-nested-027.html",
        "css/css-multicol/multicol-nested-029.html",
        "css/css-multicol/multicol-nested-033.html",
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

/// Keep the list-item marker/spanner candidate pixel-exact at 800x600.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn multicol_list_item_006_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-multicol/multicol-list-item-006.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .expect("discover multicol-list-item-006");
    assert_eq!(
        pairs.len(),
        1,
        "expected one pair for multicol-list-item-006"
    );
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run multicol-list-item-006");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "multicol-list-item-006: {:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
}
