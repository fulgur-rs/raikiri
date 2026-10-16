//! Focused CSS Text Decoration visual checks.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Verify underline, overline, and line-through propagation at the exact viewport.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_line_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text-decor/text-decoration-line.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .expect("discover CSS Text Decoration WPT pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run CSS Text Decoration WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{}: {:?} ({} mismatched pixels)",
        test.display(),
        result.outcome,
        result.mismatched_pixels
    );
}

/// Verify the initial horizontal inset implementation against the simple WPT case.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_inset_px_endpoint_case_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text-decor/text-decoration-inset-001.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .expect("discover CSS Text Decoration inset pair");
    assert_eq!(pairs.len(), 1, "{}", test.display());
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result =
        run_pair_with_images(&pairs[0], config).expect("run CSS Text Decoration inset WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{}: {:?} ({} mismatched pixels)",
        test.display(),
        result.outcome,
        result.mismatched_pixels
    );
}

/// Verify large endpoint insets with an explicit single-line precondition.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_decoration_inset_large_endpoint_nowrap_case_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text-decor/text-decoration-inset-016.html");
    let mut pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .expect("discover CSS Text Decoration inset pair");
    assert_eq!(pairs.len(), 1, "{}", test.display());

    // The reference translates headings by 10em and 12em and assumes one line.
    // The deterministic Ahem fallback is wider than a browser's usual serif font:
    // correct abspos shrink-to-fit now wraps those translated reference headings.
    // Keep both halves on one line so this fixture isolates decoration endpoints.
    let fixture = tempfile::tempdir().expect("create single-line decoration fixture");
    let single_line = |path: &std::path::Path| {
        let html = std::fs::read_to_string(path).expect("read upstream decoration document");
        format!("{html}\n<style>h1 {{ white-space: nowrap; }}</style>")
    };
    let test_html = single_line(&pairs[0].test);
    let reference_html = single_line(&pairs[0].reference);
    let fixture_test = fixture.path().join("test.html");
    let fixture_reference = fixture.path().join("reference.html");
    std::fs::write(&fixture_test, &test_html).expect("write decoration test");
    std::fs::write(&fixture_reference, reference_html).expect("write decoration reference");
    pairs[0].test = fixture_test;
    pairs[0].reference = fixture_reference;

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run single-line inset pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{:?}",
        result.outcome
    );

    // The positive pair must still detect a missing endpoint inset.
    let without_inset = format!("{test_html}<style>u {{ text-decoration-inset: 0; }}</style>");
    let negative =
        raikiri_wpt::reftest::run_pair_with_test_html(&pairs[0], config, true, "", &without_inset)
            .expect("run zero-inset negative control");
    assert!(matches!(negative.outcome, TestOutcome::Fail(_)));
    assert!(negative.mismatched_pixels > 0);
}

/// Verify fixed `text-underline-offset` alignment in the horizontal case.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_underline_offset_px_case_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text-decor/text-underline-offset-002.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .expect("discover CSS Text Decoration underline-offset pair");
    assert_eq!(pairs.len(), 1, "{}", test.display());
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config)
        .expect("run CSS Text Decoration underline-offset WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{}: {:?} ({} mismatched pixels)",
        test.display(),
        result.outcome,
        result.mismatched_pixels
    );
}

/// Verify percentage `text-underline-offset` at the exact viewport.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_underline_offset_percentage_case_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text-decor/text-underline-offset-percentage.html");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .expect("discover CSS Text Decoration percentage underline-offset pair");
    assert_eq!(pairs.len(), 1, "{}", test.display());
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config)
        .expect("run CSS Text Decoration percentage underline-offset WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "{}: {:?} ({} mismatched pixels)",
        test.display(),
        result.outcome,
        result.mismatched_pixels
    );
}

/// A percentage `text-underline-offset` inherits as a relative value, so a
/// descendant with a larger font size draws its underline further away than
/// the declaring ancestor's font size would imply.
#[test]
fn inherited_text_underline_offset_percentage_scales_with_font_size() {
    const STYLE: &str = "<style>body{margin:0}span{text-decoration:underline;\
        text-decoration-color:black;text-decoration-thickness:4px;color:transparent}</style>";
    const TEST_BODY: &str = r#"<p style="font-size:10px;text-underline-offset:50%"><span style="font-size:40px">X</span></p>"#;
    let dir = tempfile::tempdir().expect("create reftest dir");
    let write = |name: &str, head: &str, body: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("<!DOCTYPE html>{head}{STYLE}{body}"))
            .expect("write fixture");
        path
    };
    write(
        "rescaled.html",
        "",
        r#"<p style="font-size:10px"><span style="font-size:40px;text-underline-offset:20px">X</span></p>"#,
    );
    write(
        "frozen.html",
        "",
        r#"<p style="font-size:10px"><span style="font-size:40px;text-underline-offset:5px">X</span></p>"#,
    );
    let mut config = ReftestConfig::default();
    config.width = 200;
    config.height = 100;
    config.tolerance = Tolerance::EXACT;
    let run = |name: &str, reference: &str| {
        let test = write(
            name,
            &format!(r#"<link rel="match" href="{reference}">"#),
            TEST_BODY,
        );
        let pairs = discover_pairs_for_file_with_wpt_root(&test, None)
            .expect("discover inherited underline-offset pair");
        assert_eq!(pairs.len(), 1);
        run_pair_with_images(&pairs[0], config)
            .expect("run inherited underline-offset pair")
            .outcome
    };
    // 50% of the span's own 40px font size, not of the declaring 10px.
    assert!(matches!(
        run("rescaled-test.html", "rescaled.html"),
        TestOutcome::Pass
    ));
    assert!(!matches!(
        run("frozen-test.html", "frozen.html"),
        TestOutcome::Pass
    ));
}
