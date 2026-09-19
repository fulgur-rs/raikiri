//! Integration tests for the meta-assert review artifact generator.

use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn generator_writes_manifest_and_does_not_change_baseline() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("css/foo/parsing")).unwrap();
    fs::write(
        root.path().join("css/foo/test.html"),
        "<html><head><meta name=\"assert\" content=\"a visible box\"></head><body><div style=\"width:20px;height:20px;background:red\"></div></body></html>",
    )
    .unwrap();
    fs::write(
        root.path().join("css/foo/parsing/parse.html"),
        "<meta name=\"assert\" content=\"a parser result\"><body>parse</body>",
    )
    .unwrap();
    let baseline = root.path().join("baseline.txt");
    fs::write(&baseline, "# header\n").unwrap();
    let output = root.path().join("review");

    Command::cargo_bin("prepare-meta-assert-review")
        .unwrap()
        .args([
            "--wpt-root",
            root.path().to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest = fs::read_to_string(output.join("manifest.jsonl")).unwrap();
    assert!(manifest.contains("css/foo/test.html"));
    assert!(!manifest.contains("css/foo/parsing/parse.html"));
    assert!(manifest.contains("\"status\":\"pending\""));
    let template = fs::read_to_string(output.join("reviews.template.jsonl")).unwrap();
    assert!(template.contains("css/foo/test.html"));
    assert!(output.join("html/css/foo/test.html").is_file());
    assert!(output.join("screenshots/css/foo/test.html.png").is_file());
    assert_eq!(fs::read_to_string(baseline).unwrap(), "# header\n");
}

#[test]
fn include_parsing_adds_parser_candidates() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("css/foo/parsing")).unwrap();
    fs::write(
        root.path().join("css/foo/parsing/parse.html"),
        "<meta name=\"assert\" content=\"a parser result\"><body>parse</body>",
    )
    .unwrap();
    let baseline = root.path().join("baseline.txt");
    fs::write(&baseline, "").unwrap();
    let output = root.path().join("review");

    Command::cargo_bin("prepare-meta-assert-review")
        .unwrap()
        .args([
            "--wpt-root",
            root.path().to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--include-parsing",
        ])
        .assert()
        .success();

    let manifest = fs::read_to_string(output.join("manifest.jsonl")).unwrap();
    assert!(manifest.contains("css/foo/parsing/parse.html"));
}
