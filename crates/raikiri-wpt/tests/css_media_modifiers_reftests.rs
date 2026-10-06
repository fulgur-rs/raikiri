//! Exact visual checks for media-type modifiers and their error handling.

use std::path::{Path, PathBuf};

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

fn assert_exact(test: &Path, wpt_root: Option<&Path>) {
    let pairs = discover_pairs_for_file_with_wpt_root(test, wpt_root)
        .expect("discover media query reftest");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run media query reftest");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{}: {:?} ({} mismatched pixels)",
        test.display(),
        result.outcome,
        result.mismatched_pixels
    );
    assert_eq!(result.mismatched_pixels, 0);
}

#[test]
fn media_type_modifiers_render_exactly_in_print() {
    let test = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/media-type-modifiers.html");
    assert_exact(&test, None);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn reserved_media_types_do_not_become_true_under_negation() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    for relative in [
        "css/mediaqueries/mq-invalid-media-type-001.html",
        "css/mediaqueries/mq-invalid-media-type-002.html",
        "css/mediaqueries/mq-invalid-media-type-003.html",
        "css/mediaqueries/mq-invalid-media-type-004.html",
        "css/mediaqueries/mq-invalid-media-type-layer-001.html",
    ] {
        assert_exact(&root.join(relative), Some(&root));
    }
}
