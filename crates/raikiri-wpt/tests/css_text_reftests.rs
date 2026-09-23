//! Focused exact WPT reftests for CSS Text residual coverage.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn overflow_wrap_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/overflow-wrap/overflow-wrap-cluster-001.html",
        "css/css-text/overflow-wrap/overflow-wrap-cluster-002.html",
        "css/css-text/overflow-wrap/overflow-wrap-min-content-size-006.html",
    ];

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for relative in candidates {
        let test = root.join(relative);
        let pairs =
            discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
        assert_eq!(pairs.len(), 1, "{relative}");
        let result = run_pair(&pairs[0], config).expect("run WPT pair");
        assert!(
            matches!(&result.outcome, TestOutcome::Pass),
            "{relative}: outcome={:?}, mismatches={}",
            result.outcome,
            result.mismatched_pixels
        );
    }
}
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn letter_spacing_ligatures_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/letter-spacing/letter-spacing-ligatures-004.html");
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
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn writing_system_font_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/writing-system/writing-system-font-001.html");
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
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_group_align_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/text-group-align/text-group-align-center-vlr.html",
        "css/css-text/text-group-align/text-group-align-center.html",
        "css/css-text/text-group-align/text-group-align-end-vlr.html",
        "css/css-text/text-group-align/text-group-align-end.html",
        "css/css-text/text-group-align/text-group-align-left-vlr.html",
        "css/css-text/text-group-align/text-group-align-left.html",
        "css/css-text/text-group-align/text-group-align-right-vlr.html",
        "css/css-text/text-group-align/text-group-align-right.html",
        "css/css-text/text-group-align/text-group-align-start-vlr.html",
        "css/css-text/text-group-align/text-group-align-start.html",
    ];
    assert_exact_passes(&root, &candidates);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_spacing_trim_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/text-spacing-trim/text-spacing-trim-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-colon-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-dot-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-fallback-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-fallback-002.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-feature-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-narrow-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-quote-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-span-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-start-oof-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-subset-001.html",
        "css/css-text/text-spacing-trim/text-spacing-trim-trim-all-001.html",
    ];
    assert_exact_passes(&root, &candidates);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn hanging_punctuation_first_ideographic_space_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/hanging-punctuation/hanging-punctuation-first-002.html");
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
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn hanging_punctuation_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/hanging-punctuation/hanging-punctuation-first-ascii-quote.html",
        "css/css-text/hanging-punctuation/hanging-punctuation-inline-001.html",
        "css/css-text/hanging-punctuation/hanging-punctuation-last-ascii-quote.html",
        "css/css-text/hanging-punctuation/hanging-punctuation-last-whitespace.html",
        "css/css-text/hanging-punctuation/hanging-punctuation-last.html",
        "css/css-text/hanging-punctuation/hanging-punctuation-with-bidi.html",
    ];
    assert_exact_passes(&root, &candidates);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_indent_current_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/text-indent/text-indent-abspos-hanging-001.html",
        "css/css-text/text-indent/text-indent-length-001.html",
        "css/css-text/text-indent/text-indent-overflow.html",
    ];
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for relative in candidates {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        assert_eq!(pairs.len(), 1, "expected one reference pair for {relative}");
        let result = run_pair_with_images(&pairs[0], config)
            .unwrap_or_else(|error| panic!("run {relative}: {error}"));
        assert!(
            matches!(&result.outcome, TestOutcome::Pass),
            "{relative}: outcome={:?}, mismatches={}",
            result.outcome,
            result.mismatched_pixels
        );
    }
}

fn assert_exact_passes(root: &std::path::Path, candidates: &[&str]) {
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for relative in candidates {
        let test = root.join(relative);
        let pairs =
            discover_pairs_for_file_with_wpt_root(&test, Some(root)).expect("discover WPT pair");
        assert_eq!(pairs.len(), 1, "{relative}");
        let result = run_pair(&pairs[0], config).expect("run WPT pair");
        assert!(
            matches!(&result.outcome, TestOutcome::Pass),
            "{relative}: outcome={:?}, mismatches={}",
            result.outcome,
            result.mismatched_pixels
        );
    }
}

fn assert_resource_exact_passes(root: &std::path::Path, candidates: &[&str]) {
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for relative in candidates {
        let test = root.join(relative);
        let pairs =
            discover_pairs_for_file_with_wpt_root(&test, Some(root)).expect("discover WPT pair");
        assert_eq!(pairs.len(), 1, "{relative}");
        let result =
            run_pair_with_images(&pairs[0], config).expect("run WPT pair with local resources");
        assert!(
            matches!(&result.outcome, TestOutcome::Pass),
            "{relative}: outcome={:?}, mismatches={}",
            result.outcome,
            result.mismatched_pixels
        );
    }
}
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn shaping_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // PASS-only subset; the remaining shaping cases are not exact yet and are
    // kept out rather than being allowed to fail this suite.
    let candidates = [
        "css/css-text/shaping/shaping-001.html",
        "css/css-text/shaping/shaping-002.html",
        "css/css-text/shaping/shaping-003.html",
        "css/css-text/shaping/shaping-008.html",
        "css/css-text/shaping/shaping-009.html",
        "css/css-text/shaping/shaping-010.html",
        "css/css-text/shaping/shaping-011.html",
        "css/css-text/shaping/shaping-014.html",
        "css/css-text/shaping/shaping-016.html",
        "css/css-text/shaping/shaping-017.html",
        "css/css-text/shaping/shaping-018.html",
        "css/css-text/shaping/shaping-020.html",
        "css/css-text/shaping/shaping-021.html",
        "css/css-text/shaping/shaping-022.html",
        "css/css-text/shaping/shaping-023.html",
        "css/css-text/shaping/shaping-024.html",
        "css/css-text/shaping/shaping-025.html",
        "css/css-text/shaping/shaping-arabic-diacritics-001.html",
    ];
    assert_exact_passes(&root, &candidates);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_autospace_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // The Ahem stylesheet is a local WPT resource, so use the resource-enabled
    // runner to exercise the same font and stylesheet inputs as the reference.
    let candidates = [
        "css/css-text/text-autospace/text-autospace-vertical-combine-001.html",
        "css/css-text/text-autospace/text-autospace-vertical-upright-001.html",
        "css/css-text/text-autospace/text-autospace-vs-001.html",
        "css/css-text/text-autospace/text-autospace-zh-001.html",
    ];
    assert_resource_exact_passes(&root, &candidates);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn boundary_shaping_unpinned_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/boundary-shaping/boundary-shaping-002.html",
        "css/css-text/boundary-shaping/boundary-shaping-006.html",
        "css/css-text/boundary-shaping/boundary-shaping-007.html",
        "css/css-text/boundary-shaping/boundary-shaping-008.html",
    ];
    assert_exact_passes(&root, &candidates);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_encoding_unpinned_diagnostics() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // These are diagnostics, not baselines: the residual non-zero values keep
    // the cross-node shaping gaps visible without accepting them as matches.
    let candidates = [
        ("css/css-text/text-encoding/shaping-join-001.html", 0),
        ("css/css-text/text-encoding/shaping-join-002.html", 0),
        ("css/css-text/text-encoding/shaping-join-003.html", 936),
        ("css/css-text/text-encoding/shaping-no-join-001.html", 0),
        ("css/css-text/text-encoding/shaping-no-join-002.html", 0),
        ("css/css-text/text-encoding/shaping-no-join-003.html", 1784),
        ("css/css-text/text-encoding/shaping-tatweel-001.html", 0),
        ("css/css-text/text-encoding/shaping-tatweel-002.html", 3899),
        ("css/css-text/text-encoding/shaping-tatweel-003.html", 0),
    ];
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for (relative, expected_mismatches) in candidates {
        let test = root.join(relative);
        let pairs =
            discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
        assert_eq!(pairs.len(), 1, "{relative}");
        let result = run_pair(&pairs[0], config).expect("run WPT pair");
        assert_eq!(result.mismatched_pixels, expected_mismatches, "{relative}");
        if expected_mismatches == 0 {
            assert!(
                matches!(&result.outcome, TestOutcome::Pass),
                "{relative}: expected exact pass, got {:?}",
                result.outcome
            );
        } else {
            assert!(
                matches!(&result.outcome, TestOutcome::Fail(_)),
                "{relative}: expected an unpinned mismatch, got {:?}",
                result.outcome
            );
        }
    }
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_encoding_join_with_resource_resolution_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/text-encoding/shaping-join-001.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run WPT pair with resources");
    assert!(
        matches!(&result.outcome, TestOutcome::Pass),
        "resource-enabled join case: outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// Verify the smallest resource-enabled exact slice across white-space and line-breaking.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn white_space_line_breaking_resource_exact_slice_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/white-space/break-spaces-002.html",
        "css/css-text/white-space/break-spaces-004.html",
        "css/css-text/white-space/break-spaces-006.html",
        "css/css-text/white-space/break-spaces-007.html",
        "css/css-text/line-breaking/line-breaking-001.html",
    ];
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    for relative in candidates {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        assert_eq!(pairs.len(), 1, "expected one pair for {relative}");
        let result = run_pair_with_images(&pairs[0], config)
            .unwrap_or_else(|error| panic!("run {relative}: {error}"));
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{relative}: {:?} ({} mismatched pixels)",
            result.outcome,
            result.mismatched_pixels
        );
    }
}

/// Inline `overflow-wrap` continues at the containing line's start.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn overflow_wrap_span_boundaries_are_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/overflow-wrap/overflow-wrap-anywhere-span-001.html",
        "css/css-text/overflow-wrap/overflow-wrap-break-word-span-001.html",
    ];
    assert_exact_passes(&root, &candidates);
}
/// Run the complete word-spacing matrix with the bundled WPT fonts.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_spacing_matrix_at_800x600_with_bundled_fonts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // Keep existing baseline controls green while exposing the one static
    // negative-length slice supported by this test. Other mismatches remain
    // diagnostic and must not be added to the baseline.
    let candidates = [
        ("css/css-text/word-spacing/word-spacing-001.html", 27200),
        ("css/css-text/word-spacing/word-spacing-002.html", 4798),
        ("css/css-text/word-spacing/word-spacing-003.html", 0),
        (
            "css/css-text/word-spacing/word-spacing-animating-font-size.html",
            1790,
        ),
        (
            "css/css-text/word-spacing/word-spacing-animating-word-spacing.html",
            0,
        ),
        (
            "css/css-text/word-spacing/word-spacing-negative-value-001.html",
            0,
        ),
        ("css/css-text/word-spacing/word-spacing-percent-001.html", 0),
    ];
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    for (relative, expected_mismatches) in candidates {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        assert_eq!(pairs.len(), 1, "expected one reference pair for {relative}");
        let result = run_pair_with_images(&pairs[0], config)
            .unwrap_or_else(|error| panic!("run bundled-font pair {relative}: {error}"));
        assert_eq!(result.mismatched_pixels, expected_mismatches, "{relative}");
        if expected_mismatches == 0 {
            assert!(
                matches!(&result.outcome, TestOutcome::Pass),
                "{relative}: expected exact pass, got {:?}",
                result.outcome
            );
        } else {
            assert!(
                matches!(&result.outcome, TestOutcome::Fail(_)),
                "{relative}: expected an unbaselined mismatch, got {:?}",
                result.outcome
            );
        }
    }
}

/// A space at an inline boundary already ends the Arabic joining context, so
/// the letters next to it keep their unjoined forms when the words sit in
/// separate inline boxes.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn boundary_shaping_does_not_join_across_spaces() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let dir = tempfile::tempdir().expect("create reftest dir");
    std::fs::copy(
        root.join("fonts/noto/NotoNaskhArabic-regular.woff2"),
        dir.path().join("naskh.woff2"),
    )
    .expect("copy Noto Naskh Arabic");
    let write = |name: &str, head: &str, body: &str| {
        let path = dir.path().join(name);
        std::fs::write(
            &path,
            format!(
                "<!DOCTYPE html><meta charset=utf-8>{head}<style>@font-face{{font-family:t;\
                 src:url(naskh.woff2)}}body{{margin:0;font:40px t}}</style><p dir=rtl>{body}</p>"
            ),
        )
        .expect("write fixture");
        path
    };
    // The references keep the same inline structure and pin the unjoined
    // forms with an explicit ZWNJ on the letters next to the space.
    let cases = [
        (
            "word",
            "هذا <span>مثال</span>",
            "هذا <span>\u{200c}مثال</span>",
        ),
        (
            "spans",
            "<span>ع</span> <span>ع</span>",
            "<span>ع\u{200c}</span> <span>\u{200c}ع</span>",
        ),
    ];
    let mut config = ReftestConfig::default();
    config.width = 400;
    config.height = 120;
    config.tolerance = Tolerance::EXACT;
    for (name, test_body, ref_body) in cases {
        write(&format!("{name}-ref.html"), "", ref_body);
        let test = write(
            &format!("{name}.html"),
            &format!(r#"<link rel="match" href="{name}-ref.html">"#),
            test_body,
        );
        let pairs = discover_pairs_for_file_with_wpt_root(&test, None).expect("discover pair");
        assert_eq!(pairs.len(), 1, "{name}");
        let result = run_pair(&pairs[0], config).expect("run pair");
        assert!(
            matches!(result.outcome, TestOutcome::Pass),
            "{name}: {:?} ({} mismatched pixels)",
            result.outcome,
            result.mismatched_pixels
        );
    }
}
