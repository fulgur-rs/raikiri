//! Focused exact WPT coverage for CSS Text `line-break: anywhere`.

use std::path::PathBuf;

use raikiri_wpt::reftest::{
    ReftestConfig, discover_pairs_for_file_with_wpt_root, run_pair, run_pair_with_images,
};
use raikiri_wpt::runner::{TestOutcome, Tolerance};

/// Run the pinned `line-break: anywhere` WPT slice against each local reference.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn line_break_anywhere_supported_registry_subset_is_pixel_exact_at_800x600() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    // Targeted adapter matrix: original 19 exact static cases plus -006/-010
    // under implementation; do not add them to the PASS baseline before exact
    // pass. Deferred -005/-009 require break-spaces; -008 needs preserved-space
    // hanging controls. The separate overrides-uax-behavior files remain included.
    // Four inline-run cases and the pinned-font U+2011 case belong to a later stage.
    let candidates = [
        "css/css-text/line-break/line-break-anywhere-002.html",
        "css/css-text/line-break/line-break-anywhere-004.html",
        "css/css-text/line-break/line-break-anywhere-006.html",
        "css/css-text/line-break/line-break-anywhere-007.html",
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
    assert_eq!(candidates.len(), 21);

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
fn line_break_anywhere_preserves_mandatory_newlines_and_pre_wrap_fallback() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let temp_dir = tempfile::tempdir_in(&target).expect("create temporary reftest directory");
    let test = temp_dir.path().join("newline-test.html");
    let reference = temp_dir.path().join("newline-reference.html");
    // These pre-wrap cases are outside the constrained NBSP subset; keep them
    // on the existing path and compare preserved-space runs directly with `normal`.
    std::fs::write(
        &test,
        "<!doctype html><meta charset=\"utf-8\"><link rel=\"match\" href=\"newline-reference.html\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:400px;line-break:anywhere}#pre-wrap,#pre-wrap-space,#pre-wrap-long,#pre-wrap-newline{white-space:pre-wrap}#pre-line{white-space:pre-line}#pre-line-tight{white-space:pre-line;width:3ch}#pre-wrap-space{width:3ch}#pre-wrap-long,#pre-wrap-newline{width:2ch}</style><div id=\"pre-wrap\" class=\"sample\">A\nB</div><div id=\"pre-line\" class=\"sample\">C\nD</div><div id=\"pre-line-tight\" class=\"sample\">ABCDEFGHI\nJKL</div><div id=\"pre-wrap-space\" class=\"sample\">XXX XX</div><div id=\"pre-wrap-long\" class=\"sample\">X   XX</div><div id=\"pre-wrap-newline\" class=\"sample\">X   \nY</div>",
    )
    .expect("write reftest");
    std::fs::write(
        &reference,
        "<!doctype html><meta charset=\"utf-8\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:400px;line-break:anywhere}#pre-wrap,#pre-wrap-space,#pre-wrap-long,#pre-wrap-newline{white-space:pre-wrap;line-break:normal}#pre-line{white-space:pre-line}#pre-line-tight{white-space:pre-line;width:3ch}#pre-wrap-space{width:3ch}#pre-wrap-long,#pre-wrap-newline{width:2ch}</style><div id=\"pre-wrap\" class=\"sample\">A\nB</div><div id=\"pre-line\" class=\"sample\">C<br>D</div><div id=\"pre-line-tight\" class=\"sample\">ABC<br>DEF<br>GHI<br>JKL</div><div id=\"pre-wrap-space\" class=\"sample\">XXX XX</div><div id=\"pre-wrap-long\" class=\"sample\">X   XX</div><div id=\"pre-wrap-newline\" class=\"sample\">X   \nY</div>",
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
        "line-break:anywhere must preserve mandatory newlines and keep pre-wrap on the existing path: {:?}",
        result.outcome
    );
}

#[test]
fn line_break_anywhere_pre_wrap_nbsp_can_break_after_internal_space() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let temp_dir = tempfile::tempdir_in(&target).expect("create temporary reftest directory");
    let test = temp_dir.path().join("pre-wrap-space-test.html");
    let reference = temp_dir.path().join("pre-wrap-space-reference.html");
    std::fs::write(
        &test,
        "<!doctype html><meta charset=\"utf-8\"><link rel=\"match\" href=\"pre-wrap-space-reference.html\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:4ch;white-space:pre-wrap;line-break:anywhere}</style><div class=\"sample\">X&nbsp;Y Z</div>",
    )
    .expect("write pre-wrap anywhere reftest");
    std::fs::write(
        &reference,
        "<!doctype html><meta charset=\"utf-8\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:4ch;white-space:pre-wrap}</style><div class=\"sample\">X&nbsp;Y <br>Z</div>",
    )
    .expect("write pre-wrap space reference");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(temp_dir.path()))
        .expect("discover pre-wrap space reftest pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run pre-wrap space reftest pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "pre-wrap anywhere must retain its preserved internal space before wrapping: {:?}",
        result.outcome
    );
}

#[test]
fn line_break_anywhere_keeps_break_spaces_on_existing_layout_path() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let temp_dir = tempfile::tempdir_in(&target).expect("create temporary reftest directory");
    let test = temp_dir.path().join("breakspaces-test.html");
    let reference = temp_dir.path().join("breakspaces-reference.html");
    std::fs::write(
        &test,
        "<!doctype html><meta charset=\"utf-8\"><link rel=\"match\" href=\"breakspaces-reference.html\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:3ch;white-space:break-spaces;line-break:anywhere}</style><div class=\"sample\">X   XX</div>",
    )
    .expect("write break-spaces reftest");
    std::fs::write(
        &reference,
        "<!doctype html><meta charset=\"utf-8\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:3ch;white-space:break-spaces;line-break:normal}</style><div class=\"sample\">X   XX</div>",
    )
    .expect("write break-spaces reference");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(temp_dir.path()))
        .expect("discover break-spaces synthetic reftest pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run break-spaces synthetic pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "unsupported break-spaces line-break:anywhere must keep the existing layout path: {:?}",
        result.outcome
    );
}

#[test]
fn line_break_anywhere_nbsp_adapter_preserves_colored_inline_background() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let temp_dir = tempfile::tempdir_in(&target).expect("create temporary reftest directory");
    let test = temp_dir.path().join("colored-inline-test.html");
    let reference = temp_dir.path().join("colored-inline-reference.html");
    std::fs::write(
        &test,
        "<!doctype html><meta charset=\"utf-8\"><link rel=\"match\" href=\"colored-inline-reference.html\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:5ch;line-break:anywhere}span{color:red;background:blue}</style><div class=\"sample\"><span>XXXX&nbsp;XXXX</span></div>",
    )
    .expect("write colored inline reftest");
    std::fs::write(
        &reference,
        "<!doctype html><meta charset=\"utf-8\"><style>html,body{margin:0}.sample{font:20px/1 monospace;width:5ch;line-break:anywhere}span{color:red;background:blue}</style><div class=\"sample\"><span>XXXX&nbsp;<br>XXXX</span></div>",
    )
    .expect("write colored inline reference");
    let pairs = discover_pairs_for_file_with_wpt_root(&test, Some(temp_dir.path()))
        .expect("discover colored inline synthetic reftest pair");
    assert_eq!(pairs.len(), 1);
    let mut config = ReftestConfig::default();
    config.width = 800;
    config.height = 600;
    config.tolerance = Tolerance::EXACT;
    let result = run_pair(&pairs[0], config).expect("run colored inline synthetic pair");
    assert!(
        matches!(result.outcome, TestOutcome::Pass),
        "line-break:anywhere NBSP adapter must preserve colored inline text and background: {:?}",
        result.outcome
    );
}
