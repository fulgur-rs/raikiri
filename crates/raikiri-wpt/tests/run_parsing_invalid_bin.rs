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

fn reporting_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let resources = dir.path().join("resources");
    fs::create_dir_all(&resources).unwrap();
    fs::write(
        resources.join("testharness.js"),
        r#"
        var results = [], callback;
        function setup() {}
        function add_completion_callback(f) { callback = f; }
        window.addEventListener('load', function() { callback(results, {status:0,message:null}); });
    "#,
    )
    .unwrap();
    let pages = dir.path().join("css/x/parsing");
    fs::create_dir_all(&pages).unwrap();
    fs::write(
        pages.join("results.html"),
        r#"
        <script src='/resources/testharness.js'></script>
        <script src='/resources/testharnessreport.js'></script><script>
        function test_invalid_value(property, value) {
            var box = document.createElement('div');
            box.style[property] = value;
            var rejected = box.style.getPropertyValue(property) === '';
            results.push({name:'duplicate name',status:rejected ? 0 : 1,
                          message:rejected ? null : 'expected rejection'});
        }
        test_invalid_value('color', 'not-a-color');
        test_invalid_value('color', 'red');
        </script>
    "#,
    )
    .unwrap();
    fs::write(
        pages.join("error.html"),
        "<script>// test_invalid_value(\nsetTimeout(function(){},40000);</script>",
    )
    .unwrap();
    dir
}

#[test]
fn bin_writes_named_assertions_page_errors_and_optional_metrics() {
    let dir = reporting_fixture();
    let output = dir.path().join("results.json");
    let assertion = Command::cargo_bin("run-parsing-invalid")
        .unwrap()
        .env("RAIKIRI_WPT_METRICS", "1")
        .arg("--results-json")
        .arg(&output)
        .arg("--wpt-root")
        .arg(dir.path())
        .assert()
        .code(0);
    let records: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(records.as_array().unwrap().len(), 2);
    assert_eq!(records[0]["test_id"], "css/x/parsing/error.html");
    assert_eq!(records[0]["error"], "aborted: virtual time limit reached");
    assert_eq!(records[0]["tests"], serde_json::json!([]));
    assert_eq!(
        records[1]["tests"],
        serde_json::json!([
            {"name":"duplicate name", "passed":true, "message":""},
            {"name":"duplicate name", "passed":false, "message":"FAIL: expected rejection"}
        ])
    );
    assert_eq!(records[1]["error"], serde_json::Value::Null);
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).unwrap();
    #[cfg(feature = "js-native")]
    assert!(stderr.contains("NATIVE_METRICS css/x/parsing/results.html"));
    #[cfg(feature = "js-wasmtime")]
    assert!(stderr.contains("WASM_METRICS css/x/parsing/results.html"));
    assert!(stderr.contains("rss_initial_kib="));
    assert!(stderr.contains("rss_steady_kib="));
}

#[test]
fn bin_reports_json_output_failure_as_a_tooling_error() {
    let dir = reporting_fixture();
    let assertion = Command::cargo_bin("run-parsing-invalid")
        .unwrap()
        .arg("--results-json")
        .arg(dir.path())
        .arg("--wpt-root")
        .arg(dir.path())
        .assert()
        .code(1);
    assert!(
        String::from_utf8(assertion.get_output().stderr.clone())
            .unwrap()
            .contains("error:")
    );
}

#[test]
fn bin_rejects_duplicate_json_paths_and_missing_json_path() {
    for args in [
        vec![
            "--results-json",
            "first.json",
            "--results-json",
            "second.json",
        ],
        vec!["--results-json", "--wpt-root", "wpt"],
    ] {
        let assertion = Command::cargo_bin("run-parsing-invalid")
            .unwrap()
            .args(args)
            .assert()
            .code(1);
        assert!(
            String::from_utf8(assertion.get_output().stderr.clone())
                .unwrap()
                .contains("--results-json")
        );
    }
}

#[test]
fn bin_help_documents_json_output_without_requiring_a_wpt_checkout() {
    let dir = tempfile::tempdir().unwrap();
    for flag in ["--help", "-h"] {
        let assertion = Command::cargo_bin("run-parsing-invalid")
            .unwrap()
            .current_dir(dir.path())
            .arg(flag)
            .assert()
            .code(0);
        let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
        assert!(stdout.contains("Usage: run-parsing-invalid"));
        assert!(stdout.contains("--results-json PATH"));
        assert!(stdout.contains("--path-prefix PATH"));
        assert!(assertion.get_output().stderr.is_empty());
    }
}
