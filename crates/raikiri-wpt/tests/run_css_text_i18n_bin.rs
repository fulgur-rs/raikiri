//! Bin-level tests for the report-only CSS Text i18n runner.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

fn write_page(wpt_root: &Path, name: &str, html: &str) {
    let path = wpt_root.join("css/css-text/i18n").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, html).unwrap();
}

fn page(body: &str, inline_script: &str) -> String {
    format!(
        "<!doctype html><html><head><style>#box {{ height: 20px; width: 40px }}</style><script src=\"/resources/testharness.js\"></script></head><body>{body}<script>{inline_script}</script></body></html>"
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
