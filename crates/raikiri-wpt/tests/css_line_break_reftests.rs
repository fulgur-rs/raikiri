//! Focused exact WPT coverage for CSS Text `line-break: anywhere`.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Run the pinned `line-break: anywhere` WPT slice against each local reference.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn line_break_anywhere_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // First-stage scope: exact static cases verified with WPT resources. The
    // wider survey has 12 unrelated abspos auto-width oracle failures, four
    // inline-run cases that need shared paragraph layout, and one case blocked
    // by U+2011 font fallback. `anywhere-003` also needs inline JavaScript, which
    // this static runner does not execute. Keep these cases visible in the linked
    // issue; do not mask or baseline them here.
    let candidates = [
        "css/css-text/line-break/line-break-anywhere-002.html",
        "css/css-text/line-break/line-break-anywhere-004.html",
        "css/css-text/line-break/line-break-anywhere-005.html",
        "css/css-text/line-break/line-break-anywhere-006.html",
        "css/css-text/line-break/line-break-anywhere-007.html",
        "css/css-text/line-break/line-break-anywhere-008.html",
        "css/css-text/line-break/line-break-anywhere-009.html",
        "css/css-text/line-break/line-break-anywhere-010.html",
        "css/css-text/line-break/line-break-anywhere-011.html",
        "css/css-text/line-break/line-break-anywhere-012.html",
        "css/css-text/line-break/line-break-anywhere-013.html",
        "css/css-text/line-break/line-break-anywhere-014.html",
        "css/css-text/line-break/line-break-anywhere-015.html",
        "css/css-text/line-break/line-break-anywhere-016.html",
        "css/css-text/line-break/line-break-anywhere-and-white-space-002.html",
        "css/css-text/line-break/line-break-anywhere-and-white-space-005.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-001.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-002.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-003.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-004.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-006.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-010.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-012.html",
        "css/css-text/line-break/line-break-anywhere-overrides-uax-behavior-015.html",
    ];
    assert_eq!(candidates.len(), 24);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    let mut failures = Vec::new();
    for relative in candidates {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        if pairs.len() != 1 {
            failures.push(format!(
                "{relative}: expected 1 match pair, found {}",
                pairs.len()
            ));
            continue;
        }
        match run_pair_with_images(&pairs[0], config) {
            Ok(result) if matches!(&result.outcome, TestOutcome::Pass) => {}
            Ok(result) => failures.push(format!(
                "{relative}: outcome={:?}, mismatched_pixels={}",
                result.outcome, result.mismatched_pixels
            )),
            Err(error) => failures.push(format!("{relative}: run error: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "line-break:anywhere WPT failures at exact 800x600:\n{}",
        failures.join("\n")
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn line_break_anywhere_multi_node_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let candidates = [
        "css/css-text/line-break/line-break-anywhere-001.html",
        "css/css-text/line-break/line-break-anywhere-017.html",
        "css/css-text/line-break/line-break-anywhere-and-white-space-004.html",
        "css/css-text/line-break/line-break-anywhere-and-white-space-007.html",
    ];
    assert_eq!(candidates.len(), 4);

    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;

    let mut failures = Vec::new();
    for relative in candidates {
        let test = root.join(relative);
        let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(&root))
            .unwrap_or_else(|error| panic!("discover {relative}: {error}"));
        if pairs.len() != 1 {
            failures.push(format!(
                "{relative}: expected 1 match pair, found {}",
                pairs.len()
            ));
            continue;
        }
        match run_pair_with_images(&pairs[0], config) {
            Ok(result) if matches!(&result.outcome, TestOutcome::Pass) => {}
            Ok(result) => failures.push(format!(
                "{relative}: outcome={:?}, mismatched_pixels={}",
                result.outcome, result.mismatched_pixels
            )),
            Err(error) => failures.push(format!("{relative}: run error: {error}")),
        }
    }
    assert!(
        failures.is_empty(),
        "line-break:anywhere multi-node WPT failures at exact 800x600:\n{}",
        failures.join("\n")
    );
}

#[test]
fn line_break_anywhere_preserves_newlines_and_prewrap_spaces() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let temp_dir = tempfile::tempdir_in(target).expect("create temporary reftest directory");
    let test = temp_dir.path().join("newline-test.html");
    let reference = temp_dir.path().join("newline-reference.html");
    std::fs::write(
        &test,
        "<!doctype html><meta charset=\"utf-8\"><link rel=\"match\" href=\"newline-reference.html\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:400px;line-break:anywhere}#pre-wrap{white-space:pre-wrap}#pre-line{white-space:pre-line}#pre-wrap-space{white-space:pre-wrap;width:3ch}#pre-wrap-many{white-space:pre-wrap;width:2ch}#pre-wrap-newline{white-space:pre-wrap;width:2ch}</style><div id=\"pre-wrap\" class=\"sample\">A\nB</div><div id=\"pre-line\" class=\"sample\">C\nD</div><div id=\"pre-wrap-space\" class=\"sample\">XXX XX</div><div id=\"pre-wrap-many\" class=\"sample\">X   XX</div><div id=\"pre-wrap-newline\" class=\"sample\">X   \nY</div>",
    )
    .expect("write reftest");
    std::fs::write(
        &reference,
        "<!doctype html><meta charset=\"utf-8\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:400px;line-break:anywhere}#pre-wrap{white-space:pre-wrap}#pre-line{white-space:pre-line}#pre-wrap-space{white-space:pre-wrap;width:3ch}#pre-wrap-many{white-space:pre-wrap;width:2ch}#pre-wrap-newline{white-space:pre-wrap;width:2ch}</style><div id=\"pre-wrap\" class=\"sample\">A<br>B</div><div id=\"pre-line\" class=\"sample\">C<br>D</div><div id=\"pre-wrap-space\" class=\"sample\">XXX<br>XX</div><div id=\"pre-wrap-many\" class=\"sample\">X<br>XX</div><div id=\"pre-wrap-newline\" class=\"sample\">X<br>Y</div>",
    )
    .expect("write reference");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(temp_dir.path()))
        .expect("discover synthetic reftest pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run synthetic reftest pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "preserved newlines and pre-wrap trailing spaces should keep their line placement with line-break:anywhere: {:?}",
        result.outcome
    );
}

#[test]
fn line_break_anywhere_multi_node_preserves_inline_color_background_and_whitespace() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let temp_dir = tempfile::tempdir_in(&target).expect("create temporary reftest directory");
    let test = temp_dir.path().join("inline-style-test.html");
    let reference = temp_dir.path().join("inline-style-reference.html");
    std::fs::write(
        &test,
        r#"<!doctype html><meta charset="utf-8"><link rel="match" href="inline-style-reference.html"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:8ch;line-break:anywhere;color:green}.accent{color:red;background:blue}</style><div class="sample">A<span class="accent">B</span> C</div>"#,
    )
    .expect("write reftest");
    std::fs::write(
        &reference,
        r#"<!doctype html><meta charset="utf-8"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:8ch;line-break:normal;color:green}.accent{color:red;background:blue}</style><div class="sample">A<span class="accent">B</span> C</div>"#,
    )
    .expect("write reference");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(temp_dir.path()))
        .expect("discover synthetic reftest pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run synthetic reftest pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "shared inline layout should preserve text color, background, and whitespace: {:?}",
        result.outcome
    );
}

#[test]
fn line_break_anywhere_preserves_whitespace_only_inline_child() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let temp_dir = tempfile::tempdir_in(&target).expect("create temporary reftest directory");
    let test = temp_dir.path().join("inline-space-test.html");
    let reference = temp_dir.path().join("inline-space-reference.html");
    std::fs::write(
        &test,
        r#"<!doctype html><meta charset="utf-8"><link rel="match" href="inline-space-reference.html"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:2ch;line-break:anywhere;color:green}</style><div class="sample"><span>A</span> <span>B</span></div>"#,
    )
    .expect("write reftest");
    std::fs::write(
        &reference,
        r#"<!doctype html><meta charset="utf-8"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:2ch;line-break:normal;color:green}</style><div class="sample"><span>A</span><br><span>B</span></div>"#,
    )
    .expect("write reference");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(temp_dir.path()))
        .expect("discover synthetic reftest pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run synthetic reftest pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "shared inline layout should retain an empty whitespace-only text node: {:?}",
        result.outcome
    );
}

#[test]
fn line_break_anywhere_fallback_keeps_whitespace_only_inline_separators() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let temp_dir = tempfile::tempdir_in(&target).expect("create temporary reftest directory");
    let test = temp_dir.path().join("inline-fallback-space-test.html");
    let reference = temp_dir.path().join("inline-fallback-space-reference.html");
    std::fs::write(
        &test,
        r#"<!doctype html><meta charset="utf-8"><link rel="match" href="inline-fallback-space-reference.html"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:100px;color:green}#tab{line-break:anywhere}#padded{line-break:anywhere;padding-left:4px}</style><div id="tab" class="sample"><span>A</span>&#9;<span>B</span></div><div id="padded" class="sample"><span>A</span> <span>B</span></div>"#,
    )
    .expect("write reftest");
    std::fs::write(
        &reference,
        r#"<!doctype html><meta charset="utf-8"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:100px;color:green}#tab{line-break:normal}#padded{line-break:normal;padding-left:4px}</style><div id="tab" class="sample"><span>A</span> <span>B</span></div><div id="padded" class="sample"><span>A</span> <span>B</span></div>"#,
    )
    .expect("write reference");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(temp_dir.path()))
        .expect("discover synthetic fallback reftest pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run synthetic fallback reftest pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "fallback shared-layout roots should preserve whitespace-only inline separators: {:?}",
        result.outcome
    );
}
