//! Exact CSS Lists reftests for shorthand and in-flow inside markers.

use std::path::PathBuf;

use raikiri_wpt::reftest::{ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

fn assert_exact(path: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let pairs = discover_pairs_for_file_with_wpt_root(&root.join(path), Some(&root))
        .expect("discover WPT pairs");
    assert!(!pairs.is_empty(), "{path}");
    for pair in pairs {
        let mut config = ReftestConfig::default();
        config.width = 800;
        config.height = 600;
        config.tolerance = Tolerance::EXACT;
        config.require_inline_fonts = true;
        let result = run_pair(&pair, config).expect("render WPT pair");
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{path}: {:?}",
            result.outcome
        );
        println!(
            "EXACT PASS {path} (800x600, bundled fonts, {} mismatched pixels)",
            result.mismatched_pixels
        );
    }
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn inside_string_marker_is_pixel_exact() {
    assert_exact("css/css-lists/list-style-type-string-001a.html");
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn list_style_shorthand_inside_string_marker_is_pixel_exact() {
    assert_exact("css/css-lists/list-style-type-string-001b.html");
}

#[test]
fn inside_image_marker_matches_inline_image_and_wraps_subsequent_lines() {
    use raikiri_wpt::reftest::run_pair_with_images;
    let dir = tempfile::tempdir().expect("fixture directory");
    let image = std::fs::File::create(dir.path().join("marker.png")).expect("image fixture");
    let mut encoder = png::Encoder::new(image, 16, 16);
    encoder.set_color(png::ColorType::Rgba);
    let mut writer = encoder.write_header().expect("PNG header");
    writer
        .write_image_data(&[0, 128, 0, 255].repeat(16 * 16))
        .expect("PNG pixels");
    writer.finish().expect("PNG finish");
    let test = dir.path().join("test.html");
    let reference = dir.path().join("reference.html");
    let style = "<style>body {margin:0} div {font:20px/1 serif;width:110px} </style>";
    std::fs::write(&test, format!("<!doctype html><link rel=match href=reference.html>{style}<div style='display:list-item;list-style:inside url(marker.png)'>First line<br>Second line wraps here</div>")).expect("test fixture");
    std::fs::write(&reference, format!("<!doctype html>{style}<div><img src='marker.png'> First line<br>Second line wraps here</div>")).expect("reference fixture");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, None).expect("discover image fixture");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    config.require_inline_fonts = true;
    let result = run_pair_with_images(&pairs[0], config).expect("render image marker fixture");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{:?}",
        result.outcome
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn inside_numbered_markers_are_pixel_exact() {
    for style in [
        "decimal",
        "decimal-leading-zero",
        "lower-latin",
        "upper-latin",
        "lower-roman",
        "upper-roman",
    ] {
        assert_exact(&format!(
            "css/css-lists/content-property/marker-text-matches-{style}.html"
        ));
    }
}
