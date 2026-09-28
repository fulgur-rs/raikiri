//! Bin-level tests for the report-only CSS Text i18n runner.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

/// Shared fake `resources/testharness.js` stand-in, covering what these pages
/// and the runner's report script use. Results are delivered on `load`, or on
/// `done()` when the page opts into explicit done.
const FAKE_HARNESS: &str = include_str!("fixtures/fake-testharness.js");

fn write_page(wpt_root: &Path, name: &str, html: &str) {
    let path = wpt_root.join("css/css-text/i18n").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, html).unwrap();
    let harness = wpt_root.join("resources/testharness.js");
    fs::create_dir_all(harness.parent().unwrap()).unwrap();
    fs::write(harness, FAKE_HARNESS).unwrap();
}

fn page(body: &str, inline_script: &str) -> String {
    format!(
        "<!doctype html><html><head><style>#box {{ height: 20px; width: 40px }}</style><script src=\"/resources/testharness.js\"></script><script src=\"/resources/testharnessreport.js\"></script></head><body>{body}<script>{inline_script}</script></body></html>"
    )
}

fn assert_clean_page(wpt_root: &Path, name: &str) {
    write_page(
        wpt_root,
        name,
        &page(
            "<div id=\"box\"></div>",
            "test(function() { assert_true(document.getElementById('box').offsetHeight === 20); }, 'box height');",
        ),
    );
}

#[test]
fn bin_uses_the_default_target_wpt_root_and_exits_zero_when_all_tests_pass() {
    let cwd = tempfile::tempdir().unwrap();
    let wpt_root = cwd.path().join("target/wpt");
    assert_clean_page(&wpt_root, "pass.html");

    let assertion = Command::cargo_bin("run-css-text-i18n")
        .unwrap()
        .current_dir(cwd.path())
        .assert()
        .code(0);
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("PASS 1/1 css/css-text/i18n/pass.html"),
        "{stdout}"
    );
    assert!(
        stdout.contains("1/1 files all-pass, 1/1 assertions pass"),
        "{stdout}"
    );
}

#[test]
fn bin_separates_assertion_failures_from_page_execution_errors() {
    let wpt_root = tempfile::tempdir().unwrap();
    write_page(
        wpt_root.path(),
        "fail.html",
        &page(
            "<div id=\"box\"></div>",
            "test(function() { assert_true(false, 'expected failure'); }, 'deliberate failure');",
        ),
    );
    write_page(
        wpt_root.path(),
        "no-test.html",
        "<!doctype html><html><head><script src=\"/resources/testharness.js\"></script></head><body></body></html>",
    );

    let assertion = Command::cargo_bin("run-css-text-i18n")
        .unwrap()
        .args(["--wpt-root", wpt_root.path().to_str().unwrap()])
        .assert()
        .code(1);
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).unwrap();
    assert!(
        stdout.contains("FAIL 0/1 css/css-text/i18n/fail.html"),
        "{stdout}"
    );
    assert!(
        stdout.contains("1 assertion failures, 1 execution errors"),
        "{stdout}"
    );
    assert!(
        stderr.contains("ERROR css/css-text/i18n/no-test.html:"),
        "{stderr}"
    );
}

#[test]
fn bin_help_succeeds_and_invalid_arguments_fail() {
    let help = Command::cargo_bin("run-css-text-i18n")
        .unwrap()
        .arg("--help")
        .assert()
        .code(0);
    assert!(
        String::from_utf8(help.get_output().stdout.clone())
            .unwrap()
            .contains("Usage: run-css-text-i18n")
    );

    let missing_value = Command::cargo_bin("run-css-text-i18n")
        .unwrap()
        .arg("--wpt-root")
        .assert()
        .code(2);
    assert!(
        String::from_utf8(missing_value.get_output().stderr.clone())
            .unwrap()
            .contains("--wpt-root requires a path")
    );

    let unknown = Command::cargo_bin("run-css-text-i18n")
        .unwrap()
        .arg("--unknown")
        .assert()
        .code(2);
    assert!(
        String::from_utf8(unknown.get_output().stderr.clone())
            .unwrap()
            .contains("unknown argument: --unknown")
    );
}

#[test]
fn bin_reports_a_missing_wpt_root_as_a_run_error() {
    let missing = TempDir::new().unwrap().path().join("not-present");
    let assertion = Command::cargo_bin("run-css-text-i18n")
        .unwrap()
        .args(["--wpt-root", missing.to_str().unwrap()])
        .assert()
        .code(2);
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("WPT testharness run failed: I/O error"),
        "{stderr}"
    );
}

#[test]
fn bin_json_retains_duplicate_names_failures_and_page_errors() {
    let dir = tempfile::tempdir().unwrap();
    write_page(
        dir.path(),
        "results.html",
        &page(
            "",
            "test(function(){assert_true(true);},'同名'); test(function(){assert_true(false,'deliberate');},'同名');",
        ),
    );
    write_page(dir.path(), "no-test.html", &page("", ""));
    let output = dir.path().join("results.json");
    Command::cargo_bin("run-css-text-i18n")
        .unwrap()
        .arg("--wpt-root")
        .arg(dir.path())
        .arg("--results-json")
        .arg(&output)
        .env("RAIKIRI_WPT_METRICS", "1")
        .assert()
        .code(1);
    let records: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(records.as_array().unwrap().len(), 2);
    assert_eq!(records[0]["test_id"], "css/css-text/i18n/no-test.html");
    assert_eq!(records[0]["tests"], serde_json::json!([]));
    assert!(
        records[0]["error"]
            .as_str()
            .unwrap()
            .contains("no test results")
    );
    assert_eq!(records[1]["error"], serde_json::Value::Null);
    assert_eq!(
        records[1]["tests"][0],
        serde_json::json!({"name":"同名","passed":true,"message":""})
    );
    assert_eq!(records[1]["tests"][1]["name"], "同名");
    assert_eq!(records[1]["tests"][1]["passed"], false);
    assert!(
        records[1]["tests"][1]["message"]
            .as_str()
            .unwrap()
            .contains("FAIL: Error: assert_true: deliberate")
    );
}

#[test]
fn bin_json_write_failure_has_a_distinct_tooling_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    assert_clean_page(dir.path(), "pass.html");
    let assertion = Command::cargo_bin("run-css-text-i18n")
        .unwrap()
        .arg("--wpt-root")
        .arg(dir.path())
        .arg("--results-json")
        .arg(dir.path())
        .assert()
        .code(2);
    assert!(
        String::from_utf8(assertion.get_output().stderr.clone())
            .unwrap()
            .contains("result output:")
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
        let assertion = Command::cargo_bin("run-css-text-i18n")
            .unwrap()
            .args(args)
            .assert()
            .code(2);
        assert!(
            String::from_utf8(assertion.get_output().stderr.clone())
                .unwrap()
                .contains("--results-json")
        );
    }
}
