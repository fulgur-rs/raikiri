//! Focused exact WPT reftests for CSS Text residual coverage.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, ReftestKind, discover_pairs_for_file_with_wpt_root, run_pair,
    run_pair_with_images, run_pair_with_variant,
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
fn text_spacing_trim_fallback_helper_highlights_exact_pass() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let relative = "css/css-text/text-spacing-trim/text-spacing-trim-fallback-001.html";
    let test = root.join(relative);
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
    assert_eq!(pairs.len(), 1, "expected one reference pair for {relative}");

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config)
        .unwrap_or_else(|error| panic!("run {relative}: {error}"));
    assert!(
        matches!(&result.outcome, TestOutcome::Pass),
        "{relative}: outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_spacing_trim_declared_variants_unpinned_exact_passes() {
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
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    let mut declared_variant_count = 0;
    let mut comparison_count = 0;
    let mut failures = Vec::new();
    for relative in candidates {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        assert_eq!(pairs.len(), 1, "expected one reference pair for {relative}");

        let html = std::fs::read_to_string(&test)
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        let variants = declared_variant_queries(&html);
        declared_variant_count += variants.len();
        let queries = if variants.is_empty() {
            vec![String::new()]
        } else {
            variants
        };

        for query in queries {
            comparison_count += 1;
            let label = if query.is_empty() {
                "<no variant>"
            } else {
                query.as_str()
            };
            match run_pair_with_variant(&pairs[0], config, &query) {
                Ok(result) if matches!(&result.outcome, TestOutcome::Pass) => {
                    println!("PASS {relative} {label}");
                }
                Ok(result) => {
                    println!(
                        "FAIL {relative} {label}: outcome={:?}, mismatches={}",
                        result.outcome, result.mismatched_pixels
                    );
                    failures.push(format!(
                        "{relative} {label}: outcome={:?}, mismatches={}",
                        result.outcome, result.mismatched_pixels
                    ));
                }
                Err(error) => {
                    println!("ERROR {relative} {label}: {error}");
                    failures.push(format!("{relative} {label}: {error}"));
                }
            }
        }
    }

    assert_eq!(
        declared_variant_count, 58,
        "update the focused WPT variant inventory"
    );
    assert_eq!(
        comparison_count, 59,
        "58 variants plus one fixture without variants"
    );
    assert!(
        failures.is_empty(),
        "failed exact comparisons:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_spacing_trim_quote_variants_exact_pass() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let relative = "css/css-text/text-spacing-trim/text-spacing-trim-quote-001.html";
    let test = root.join(relative);
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
        .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
    assert_eq!(pairs.len(), 1, "expected one reference pair for {relative}");

    let html =
        std::fs::read_to_string(&test).unwrap_or_else(|error| panic!("read {relative}: {error}"));
    let queries = declared_variant_queries(&html);
    assert_eq!(queries.len(), 9, "update the quote-001 variant inventory");

    let mut failures = Vec::new();
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    for query in queries {
        match run_pair_with_variant(&pairs[0], config, &query) {
            Ok(result) if matches!(&result.outcome, TestOutcome::Pass) => {
                println!("PASS {relative} {query}");
            }
            Ok(result) => failures.push(format!(
                "{relative} {query}: outcome={:?}, mismatches={}",
                result.outcome, result.mismatched_pixels
            )),
            Err(error) => failures.push(format!("{relative} {query}: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "failed exact comparisons:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_spacing_trim_vertical_variants_exact_pass() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let variants = [
        (
            "css/css-text/text-spacing-trim/text-spacing-trim-feature-001.html",
            "?class=vrl&feature=vhal",
        ),
        (
            "css/css-text/text-spacing-trim/text-spacing-trim-feature-001.html",
            "?class=vrl&feature=vpal",
        ),
        (
            "css/css-text/text-spacing-trim/text-spacing-trim-trim-all-001.html",
            "?class=halt,vrl",
        ),
        (
            "css/css-text/text-spacing-trim/text-spacing-trim-trim-all-001.html",
            "?class=chws,vrl",
        ),
    ];

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let mut failures = Vec::new();
    for (relative, query) in variants {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        assert_eq!(pairs.len(), 1, "expected one reference pair for {relative}");
        let result = run_pair_with_variant(&pairs[0], config, query)
            .unwrap_or_else(|error| panic!("run {relative} {query}: {error}"));
        if matches!(&result.outcome, TestOutcome::Pass) {
            println!("PASS {relative} {query}");
        } else {
            println!(
                "FAIL {relative} {query}: outcome={:?}, mismatches={}",
                result.outcome, result.mismatched_pixels
            );
            failures.push(format!(
                "{relative} {query}: outcome={:?}, mismatches={}",
                result.outcome, result.mismatched_pixels
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "failed exact comparisons:\n{}",
        failures.join("\n")
    );
}

fn declared_variant_queries(html: &str) -> Vec<String> {
    let parsed = raikiri_html::parse(
        html.as_bytes(),
        &raikiri::ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .expect("parse WPT variant metadata");
    let document = &parsed.dom;
    let mut variants = Vec::new();
    let mut stack = vec![document.root_index()];
    while let Some(node_id) = stack.pop() {
        let Some(node) = document.get_node(node_id) else {
            continue;
        };
        if node.tag_name() == Some("meta")
            && node
                .attribute("name")
                .is_some_and(|name| name.eq_ignore_ascii_case("variant"))
            && let Some(content) = node.attribute("content")
        {
            variants.push(content.to_owned());
        }
        stack.extend(node.children.iter().rev().copied());
    }
    variants
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

/// Percentage text-indent resolves against the content box, including the
/// `overflow:hidden` clip edge in 003 and the `calc()` basis in 004.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_indent_percentage_content_box_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/text-indent/text-indent-percentage-002.html",
        "css/css-text/text-indent/text-indent-percentage-003.html",
        "css/css-text/text-indent/text-indent-percentage-004.html",
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

/// Inline element boundaries preserve ideograph-to-Latin auto spacing.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_autospace_inline_element_boundaries_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/text-autospace/text-autospace-elements-005.html",
        "css/css-text/text-autospace/text-autospace-elements-005b.html",
    ];
    assert_resource_exact_passes(&root, &candidates);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_autospace_resource_exact_passes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // The Ahem stylesheet is a local WPT resource, and the vs/zh references
    // use nested CSS rules, so run with local resources enabled.
    let candidates = [
        "css/css-text/text-autospace/text-autospace-ideogram-alpha-001.html",
        "css/css-text/text-autospace/text-autospace-ligature-001.html",
        "css/css-text/text-autospace/text-autospace-vertical-combine-001.html",
        "css/css-text/text-autospace/text-autospace-vertical-upright-001.html",
        "css/css-text/text-autospace/text-autospace-vs-001.html",
        "css/css-text/text-autospace/text-autospace-zh-001.html",
        // Nested inline wrappers must remain on one synthetic line box while
        // autospace reserves its boundary advances.
        "css/css-text/text-autospace/text-autospace-elements-005.html",
        "css/css-text/text-autospace/text-autospace-elements-005b.html",
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
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    for relative in [
        "css/css-text/text-encoding/shaping-join-001.html",
        "css/css-text/text-encoding/shaping-join-002.html",
    ] {
        let test = root.join(relative);
        let pairs =
            discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
        assert_eq!(pairs.len(), 1, "{relative}");
        let result = run_pair_with_images(&pairs[0], config).expect("run WPT pair with resources");
        assert!(
            matches!(&result.outcome, TestOutcome::Pass),
            "{relative}: outcome={:?}, mismatches={}",
            result.outcome,
            result.mismatched_pixels
        );
    }
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
    // Pin each file's exact pixel outcome. Track mismatches as exact expected
    // failures; render and harness errors remain hard failures.
    let candidates = [
        ("css/css-text/word-spacing/word-spacing-001.html", 4000),
        ("css/css-text/word-spacing/word-spacing-002.html", 0),
        ("css/css-text/word-spacing/word-spacing-003.html", 0),
        (
            "css/css-text/word-spacing/word-spacing-animating-font-size.html",
            0,
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

/// Both animation fixtures must pass at exact pixel tolerance.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_spacing_animation_wpts_pass_exactly_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/word-spacing/word-spacing-animating-font-size.html",
        "css/css-text/word-spacing/word-spacing-animating-word-spacing.html",
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
            .unwrap_or_else(|error| panic!("run bundled-font pair {relative}: {error}"));
        assert_eq!(result.mismatched_pixels, 0, "{relative}");
        assert!(
            matches!(&result.outcome, TestOutcome::Pass),
            "{relative}: expected exact pass, got {:?}",
            result.outcome
        );
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

/// Preserve a trailing newline without treating its letter spacing as a wrap.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn letter_spacing_preserved_newline_does_not_wrap() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/letter-spacing/letter-spacing-end-of-line-002.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// An explicit script tag overrides Turkish casing for CSS lowercase.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn script_tag_lowercase_overrides_turkish_casing_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/writing-system/writing-system-text-transform-001.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// Basic `overflow-wrap: break-word` wraps a long Ahem token inside its box.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn overflow_wrap_basic_break_word_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/overflow-wrap/overflow-wrap-001.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run WPT pair with Ahem font");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// `white-space: nowrap` suppresses `overflow-wrap: break-word` on the blue box.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn overflow_wrap_break_word_does_not_override_nowrap_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/overflow-wrap/overflow-wrap-002.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run WPT pair with Ahem font");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// `text-justify: none` leaves Ahem word spacing unchanged under justify alignment.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_justify_none_disables_expansion_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/text-justify/text-justify-001.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run WPT pair with Ahem font");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// `word-space-transform: space` turns ZWSP and `<wbr>` into ASCII spaces.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_space_transform_space_for_zwsp_and_wbr_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/word-space-transform/word-space-transform-002.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// `word-space-transform: ideographic-space` expands ZWSP and `<wbr>` to U+3000.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_space_transform_ideographic_space_for_zwsp_and_wbr_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/word-space-transform/word-space-transform-001.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

fn word_space_transform_none_override_pair(file: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/word-space-transform").join(file);
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// The `<wbr>` may opt out of its parent's `word-space-transform: space`.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_space_transform_none_on_wbr_exact() {
    word_space_transform_none_override_pair("word-space-transform-004.html");
}

/// An inline span may opt out of its parent's `word-space-transform: space`.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_space_transform_none_on_inline_exact() {
    word_space_transform_none_override_pair("word-space-transform-005.html");
}

/// `word-space-transform: space` on inline children works without a parent value.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_space_transform_space_on_inline_children_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/word-space-transform/word-space-transform-006.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// Without `auto-phrase`, ideographic spaces do not appear inside Japanese words.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn word_space_transform_without_auto_phrase_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/word-space-transform/word-space-transform-028.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// `no-autospace` differs from `normal` in both WPT match and mismatch pairs.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_autospace_no_vs_normal_both_pairs_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/text-autospace/text-autospace-no-001.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pairs");
    assert_eq!(
        pairs.len(),
        2,
        "exercise both match and mismatch references"
    );
    assert_eq!(
        pairs
            .iter()
            .filter(|pair| pair.kind == ReftestKind::Match)
            .count(),
        1
    );
    assert_eq!(
        pairs
            .iter()
            .filter(|pair| pair.kind == ReftestKind::Mismatch)
            .count(),
        1
    );

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let mut failures = Vec::new();
    for (index, pair) in pairs.iter().enumerate() {
        let result = run_pair(pair, config).expect("run WPT pair");
        eprintln!(
            "pair {index} {:?}: mismatches={}",
            pair.kind, result.mismatched_pixels
        );
        if !matches!(result.outcome, TestOutcome::Pass) {
            failures.push(format!(
                "pair {index}: outcome={:?}, mismatches={}",
                result.outcome, result.mismatched_pixels
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

/// A numeric/ideograph boundary receives one eighth em of Ahem spacing.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_autospace_ideograph_numeric_ahem_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/text-autospace/text-autospace-ideograph-numeric-001.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run WPT pair with Ahem font");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// Autospace recognizes a supplementary CJK ideograph at Latin boundaries.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_autospace_supplementary_ideograph_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/text-autospace/text-autospace-supplementary-ideograph.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// Full-width conversion applies to only the surviving collapsed space.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_transform_fullwidth_collapsed_spaces_ahem_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/text-transform/text-transform-fullwidth-006.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run WPT pair with Ahem");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// Full-width conversion applies to each preserved space in `pre-wrap`.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_transform_fullwidth_preserved_spaces_ahem_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/text-transform/text-transform-fullwidth-007.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair_with_images(&pairs[0], config).expect("run WPT pair with Ahem");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}

/// Unicode wide/narrow compatibility forms use their full-width mappings.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn text_transform_fullwidth_unicode_mapping_exact() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let test = root.join("css/css-text/text-transform/text-transform-fullwidth-010.html");
    let pairs =
        discover_pairs_for_file_with_wpt_root(&test, Some(&root)).expect("discover WPT pair");
    assert_eq!(pairs.len(), 1);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run WPT pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "outcome={:?}, mismatches={}",
        result.outcome,
        result.mismatched_pixels
    );
}
