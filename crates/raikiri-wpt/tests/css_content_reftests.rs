//! Focused WPT reftests for literal generated content.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn before_after_literal_strings_are_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/CSS2/generated-content/before-after-002.xht");
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
        "outcome: {:?}",
        result.outcome
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_text_control_char_literal_generated_content_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/white-space/control-chars-000.html");
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
        "outcome: {:?}",
        result.outcome
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_values_attr_notype_fallback_generated_content_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-values/attr-notype-fallback.html");
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
        "outcome: {:?}",
        result.outcome
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_page_quote_content_list_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-page/margin-boxes/content-002-print.html");
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
        "outcome: {:?}",
        result.outcome
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css2_missing_image_generated_content_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/CSS2/generated-content/before-after-images-001.xht");
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
        "outcome: {:?}",
        result.outcome
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css2_counter_generated_content_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/CSS2/generated-content/content-005.xht");
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
        "outcome: {:?}",
        result.outcome
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css2_counter_auto_reset_generated_content_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/CSS2/generated-content/content-auto-reset-001.xht");
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
        "outcome: {:?}",
        result.outcome
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css2_counter_increment_generated_content_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/CSS2/generated-content/content-counter-000.xht");
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
        "outcome: {:?}",
        result.outcome
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css2_counters_generated_content_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/CSS2/generated-content/content-021.xht");
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
        "outcome: {:?}",
        result.outcome
    );
}

/// Measure page image painting cases after the image-resource changes. These
/// remain diagnostics until every resource path is pixel-exact.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_page_image_cases_are_remeasured_without_baselining() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let cases = [
        "css/css-page/background-image-only-for-print.html",
        "css/css-page/page-background-image-print.html",
        "css/css-page/page-background-005-print.html",
        "css/css-page/margin-boxes/background-001-print.html",
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
        match run_pair_with_images(&pairs[0], config) {
            Ok(result) => eprintln!(
                "{relative}: {:?} ({} mismatched pixels); diagnostic only",
                result.outcome, result.mismatched_pixels
            ),
            Err(error) => eprintln!("{relative}: unsupported ({error}); diagnostic only"),
        }
    }
}

/// A forced page break must produce the second printed page, not just move
/// content inside the first page.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn css_break_forced_page_break_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-break/zero-height-page-break-001-print.html");
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
        "outcome: {:?} ({} mismatched pixels)",
        result.outcome,
        result.mismatched_pixels
    );
}
