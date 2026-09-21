//! Focused WPT reftests for the CSS Display foundation.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Run the standalone `display: flow-root` WPT pair at the pinned 800x600
/// viewport. Fetch the sparse WPT checkout first with `scripts/wpt/fetch.sh`.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn display_flow_root_002_is_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-display/display-flow-root-002.html");
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

/// Run two `display: contents` alignment pairs at the pinned 800x600 viewport.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn display_contents_alignment_pairs_are_pixel_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for name in [
        "display-contents-alignment-001.html",
        "display-contents-alignment-002.html",
    ] {
        let test = root.join("css/css-display").join(name);
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
