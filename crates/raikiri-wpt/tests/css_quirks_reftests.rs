//! Exact local reftest for the quirks-mode inline line-height calculation.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

#[test]
fn text_free_inline_struts_match_direct_replaced_children() {
    let test = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/quirks-inline-strut.html");
    let pairs = discover_pairs_for_file(&test).expect("discover local quirks reftest");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run local quirks reftest");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
    assert_eq!(result.mismatched_pixels, 0);
}

#[test]
fn line_height_quirk_flag_pair() {
    let test = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/quirks-strut-flag.html");
    let pairs = discover_pairs_for_file(&test).expect("discover local quirks reftest");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run local quirks reftest");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
    assert_eq!(result.mismatched_pixels, 0);
}

#[test]
fn line_height_quirk_br_pair() {
    let test = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/quirks-strut-br.html");
    let pairs = discover_pairs_for_file(&test).expect("discover local quirks reftest");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run local quirks reftest");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
    assert_eq!(result.mismatched_pixels, 0);
}

#[test]
fn line_height_quirk_br_own_style_pair() {
    let test =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/quirks-strut-br-own-style.html");
    let pairs = discover_pairs_for_file(&test).expect("discover local quirks reftest");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run local quirks reftest");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
    assert_eq!(result.mismatched_pixels, 0);
}
