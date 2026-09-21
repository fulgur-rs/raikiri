//! Focused running-element generated-content checks.
use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::TestOutcome;
use std::path::PathBuf;
#[test]
fn running_element_margin_box_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata");
    let test = root.join("running-simple.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root)).unwrap();
    assert_eq!(pairs.len(), 1);
    let mut c = ReftestConfig::default();
    c.width = 800;
    c.height = 600;
    let r = run_pair(&pairs[0], c).unwrap();
    assert!(
        matches!(r.outcome, TestOutcome::Pass),
        "{test:?}: {:?} ({} mismatched pixels)",
        r.outcome,
        r.mismatched_pixels
    );
}
