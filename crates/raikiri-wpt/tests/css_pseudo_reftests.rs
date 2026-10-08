//! Focused CSS Pseudo visual checks at the exact project viewport.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Real non-floating first-letter pairs and an active-selection compatibility guard.
/// The selection pair has no visible ink and does not prove selection painting.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn pseudo_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let dir = root.join("css/css-pseudo");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for name in [
        "active-selection-056.html",
        "first-letter-004.html",
        "first-letter-005.html",
        "first-letter-with-before-after.html",
    ] {
        let test = dir.join(name);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
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
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn floating_first_letter_is_explicitly_unsupported_until_drop_cap_layout_lands() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-pseudo/first-letter-003.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root)).unwrap();
    assert_eq!(pairs.len(), 1);
    let error = match run_pair(&pairs[0], ReftestConfig::default()) {
        Err(error) => error,
        Ok(_) => panic!("a floated first-letter requires real drop-cap layout"),
    };
    assert!(
        error
            .to_string()
            .contains("floating ::first-letter requires drop-cap box layout"),
        "{error}"
    );
}
