//! Focused visual checks for image-related CSS behavior.

use std::path::Path;
use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::TestOutcome;

fn run_exact(root: &Path, relative: &str) {
    let test = root.join(relative);
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(root))
        .unwrap_or_else(|error| panic!("discover {}: {error}", test.display()));
    assert_eq!(pairs.len(), 1, "expected one pair for {}", test.display());
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
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

/// The supported intrinsic aspect-ratio path stays pixel exact.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn aspect_ratio_intrinsic_size_is_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    run_exact(
        &root,
        "css/css-flexbox/aspect-ratio-intrinsic-size-007.html",
    );
}

/// Print-only background images match their references exactly.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn print_background_images_are_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    for relative in [
        "css/css-page/background-image-only-for-print.html",
        "css/css-page/page-background-image-print.html",
    ] {
        run_exact(&root, relative);
    }
}

/// A URL-valued list-style-image is decoded and painted as the marker image.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn list_style_image_is_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    run_exact(
        &root,
        "css/css-images/image-orientation/image-orientation-list-style-image.html",
    );
}
