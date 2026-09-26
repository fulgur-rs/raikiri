//! Bin-level integration test for `run-parsing-invalid`.

use std::fs;
use std::path::Path;

use assert_cmd::Command;

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn bin_reports_box_sizing_invalid_as_all_pass() {
    // `cargo test`'s working directory is this crate's manifest directory,
    // not the workspace root, so `target/wpt` needs the same absolute-path
    // treatment `parsing_invalid`'s own tests use.
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let assert = Command::cargo_bin("run-parsing-invalid")
        .unwrap()
        .arg("--wpt-root")
        .arg(&wpt_root)
        .args(["--path-prefix", "css/css-sizing/parsing/box-sizing-invalid"])
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        out.contains("PASS 7/7 css/css-sizing/parsing/box-sizing-invalid.html"),
        "unexpected stdout: {out}"
    );
}

/// The `main` loop's `Err(ParsingFileError::NoParsingTestCalls) => continue`
/// arm (a file with neither a `test_invalid_value(` nor a `test_valid_value(`
/// call is skipped outright, never printed): a synthetic single-file WPT
/// root, not the real sparse checkout, is enough to exercise it, since
/// `run_parsing_invalid_file` returns that error before it ever reads
/// `css/support/parsing-testcommon.js`.
#[test]
fn bin_skips_a_parsing_file_with_no_parsing_test_calls() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("css/x/parsing/none.html");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "<script>test_valid_selector(\"div\");</script>").unwrap();
    let assert = Command::cargo_bin("run-parsing-invalid")
        .unwrap()
        .arg("--wpt-root")
        .arg(dir.path())
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        out.contains("-- 0/0 files all-pass, 0/0 assertions pass --"),
        "unexpected stdout: {out}"
    );
    assert!(!out.contains("none.html"), "unexpected stdout: {out}");
}
