//! Focused CSS Images visual checks.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// `object-fit` paints an RGB PNG into the replaced content box for each
/// concrete-object sizing mode at the project-wide exact viewport.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn object_fit_png_pairs_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    for mode in ["fill", "contain", "cover", "none", "scale-down"] {
        let test = root.join(format!("css/css-images/object-fit-{mode}-png-001i.html"));
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {}: {error}", test.display()));
        assert_eq!(pairs.len(), 1, "expected one pair for {}", test.display());
        let result = run_pair_with_images(&pairs[0], config)
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

/// Basic gradient interpolation with a transparent stop should match the
/// equivalent explicit transparent blue stop.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn gradients_with_transparent_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-images/gradients-with-transparent.html");
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

/// A repeated linear gradient must honor the border-box background origin.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn gradient_border_box_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-images/gradient-border-box.html");
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

/// The default image-orientation case is measured separately because EXIF
/// rotation is not yet part of the PNG-only image scope.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn image_orientation_default_is_measured_without_baselining() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-images/image-orientation/image-orientation-default.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    match run_pair_with_images(&pairs[0], config) {
        Ok(result) => eprintln!(
            "{}: {:?} ({} mismatched pixels); diagnostic only",
            test.display(),
            result.outcome,
            result.mismatched_pixels
        ),
        Err(error) => eprintln!("{}: unsupported ({error}); diagnostic only", test.display()),
    }
}

/// Measure the PNG `contain-intrinsic-size` image case without accepting an
/// implementation result as a baseline.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn contain_intrinsic_size_png_is_measured_without_baselining() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test =
        root.join("css/css-images/object-fit-containcontainintrinsicsize-png-001i.tentative.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    match run_pair_with_images(&pairs[0], config) {
        Ok(result) => eprintln!(
            "{}: {:?} ({} mismatched pixels); diagnostic only",
            test.display(),
            result.outcome,
            result.mismatched_pixels
        ),
        Err(error) => eprintln!("{}: unsupported ({error}); diagnostic only", test.display()),
    }
}
