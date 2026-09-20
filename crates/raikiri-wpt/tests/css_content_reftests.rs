//! Focused WPT reftests for literal generated content.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
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
