//! Integration tests for the dashboard generator binary.

use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn binary_generates_dashboard_from_a_non_git_wpt_tree() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("css/foo/parsing")).unwrap();
    fs::write(root.path().join("css/foo/a.html"), "a").unwrap();
    fs::write(root.path().join("css/foo/parsing/b.html"), "b").unwrap();
    fs::write(root.path().join("css/README.md"), "readme").unwrap();

    let baseline = root.path().join("baseline.txt");
    fs::write(
        &baseline,
        "# comment\ncss/foo/a.html\nb.html\ncss/README.md\n",
    )
    .unwrap();
    let output = root.path().join("nested/dashboard.html");

    let assertion = Command::cargo_bin("generate-dashboard")
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
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("3 WPT files, 3 baseline entries"));

    let html = fs::read_to_string(output).unwrap();
    assert!(html.contains("Total (css/)") && html.contains("3/3"));
    assert!(html.contains("css/foo/parsing") && html.contains("1/1"));
}

#[test]
fn binary_help_and_invalid_input_are_reported() {
    let help = Command::cargo_bin("generate-dashboard")
        .unwrap()
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("generate-dashboard"));

    let invalid = Command::cargo_bin("generate-dashboard")
        .unwrap()
        .args(["--wpt-root", "/path/that/does/not/exist"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("could not enumerate"));
}

#[test]
fn binary_reports_output_path_errors() {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("css/foo")).unwrap();
    fs::write(root.path().join("css/foo/a.html"), "a").unwrap();
    let baseline = root.path().join("baseline.txt");
    fs::write(&baseline, "css/foo/a.html\n").unwrap();

    let parent_file = root.path().join("parent-file");
    fs::write(&parent_file, "not a directory").unwrap();
    let parent_error = Command::cargo_bin("generate-dashboard")
        .unwrap()
        .args([
            "--wpt-root",
            root.path().to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--output",
            parent_file.join("dashboard.html").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!parent_error.status.success());
    assert!(
        String::from_utf8_lossy(&parent_error.stderr)
            .contains("could not create dashboard output directory")
    );

    let output_dir = root.path().join("output-dir");
    fs::create_dir(&output_dir).unwrap();
    let output_error = Command::cargo_bin("generate-dashboard")
        .unwrap()
        .args([
            "--wpt-root",
            root.path().to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output_error.status.success());
    assert!(String::from_utf8_lossy(&output_error.stderr).contains("could not write"));

    let no_parent = Command::cargo_bin("generate-dashboard")
        .unwrap()
        .current_dir(root.path())
        .args([
            "--wpt-root",
            root.path().to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--output",
            "dashboard.html",
        ])
        .output()
        .unwrap();
    assert!(no_parent.status.success());
    assert!(root.path().join("dashboard.html").is_file());
}

#[test]
fn binary_reports_argument_errors() {
    let output = Command::cargo_bin("generate-dashboard")
        .unwrap()
        .arg("--unknown")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown argument"));
}
