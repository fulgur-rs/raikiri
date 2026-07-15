//! Bin-level integration test for `validate-expectations`.
//!
//! Uses `assert_cmd` to invoke the compiled bin against a `tempfile`-
//! staged expectations directory via the `RAIKIRI_WPT_EXPECTATIONS_DIR`
//! env var override.

use std::fs;

use assert_cmd::Command;
use tempfile::TempDir;

fn stage_header_only_dir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "tracked-wpt.txt",
        "known-issues.txt",
        "raikiri-baseline.txt",
        "quarantine.txt",
        "deprecated.txt",
    ] {
        fs::write(dir.path().join(name), "# header\n").unwrap();
    }
    dir
}

#[test]
fn bin_exits_zero_on_clean_expectations() {
    let dir = stage_header_only_dir();
    let assert = Command::cargo_bin("validate-expectations")
        .unwrap()
        .env("RAIKIRI_WPT_EXPECTATIONS_DIR", dir.path())
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("clean"), "unexpected stdout: {out}");
}

#[test]
fn bin_exits_two_on_malformed_quarantine() {
    let dir = stage_header_only_dir();
    // Overwrite quarantine.txt with a malformed line (3 cols instead of 8).
    fs::write(
        dir.path().join("quarantine.txt"),
        "css/foo | linux | x86_64\n",
    )
    .unwrap();
    let assert = Command::cargo_bin("validate-expectations")
        .unwrap()
        .env("RAIKIRI_WPT_EXPECTATIONS_DIR", dir.path())
        .assert()
        .code(2);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("Malformed"), "unexpected stdout: {out}");
}

#[test]
fn bin_exits_zero_when_only_expired_warnings() {
    let dir = stage_header_only_dir();
    // A quarantine entry with a very old added_date (well beyond 90 days,
    // relative to any plausible bin-invocation "now").
    fs::write(
        dir.path().join("quarantine.txt"),
        "css/foo | linux | x86_64 | vello_cpu | low | r | i | 2000-01-01\n",
    )
    .unwrap();
    let assert = Command::cargo_bin("validate-expectations")
        .unwrap()
        .env("RAIKIRI_WPT_EXPECTATIONS_DIR", dir.path())
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.contains("Expired"), "unexpected stdout: {out}");
    assert!(out.contains("warning(s)"), "unexpected stdout: {out}");
}
