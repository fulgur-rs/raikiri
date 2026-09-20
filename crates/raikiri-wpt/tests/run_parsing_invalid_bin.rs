//! Bin-level integration test for `run-parsing-invalid`.

use std::path::Path;

use assert_cmd::Command;

#[test]
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
