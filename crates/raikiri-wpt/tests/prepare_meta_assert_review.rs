//! Integration tests for the meta-assert review artifact generator.

use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

#[cfg(unix)]
#[test]
fn unix_filename_characters_cannot_redirect_artifact_writes() {
    let root = tempdir().unwrap();
    let wpt = root.path().join("wpt");
    let output = root.path().join("review");
    let names = [
        r"css/..\..\..\victim.html",
        r"css/..\..\..\escape/test.html",
        r"css/a\b.html",
        "css/a/b.html",
    ];
    for (index, name) in names.iter().enumerate() {
        let path = wpt.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!("<meta name=assert content='fixture {index}'><body></body>"),
        )
        .unwrap();
    }
    let baseline = root.path().join("baseline.txt");
    fs::write(&baseline, "").unwrap();
    let victim = root.path().join("victim.html");
    let png_victim = root.path().join("victim.html.png");
    fs::write(&victim, "HTML sentinel").unwrap();
    fs::write(&png_victim, "PNG sentinel").unwrap();
    Command::cargo_bin("prepare-meta-assert-review")
        .unwrap()
        .arg("--wpt-root")
        .arg(&wpt)
        .arg("--baseline")
        .arg(&baseline)
        .arg("--exclude-baseline")
        .arg(&baseline)
        .arg("--output")
        .arg(&output)
        .args(["--width", "1", "--height", "1"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(victim).unwrap(), "HTML sentinel");
    assert_eq!(fs::read_to_string(png_victim).unwrap(), "PNG sentinel");
    assert!(!root.path().join("escape").exists());
    let manifest = fs::read_to_string(output.join("manifest.jsonl")).unwrap();
    let entries: Vec<serde_json::Value> = manifest
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(entries.len(), names.len());
    for name in names {
        let entry = entries
            .iter()
            .find(|entry| entry["test_id"] == name)
            .unwrap();
        let html = entry["html"].as_str().unwrap();
        let screenshot = entry["screenshot"].as_str().unwrap();
        assert_eq!(html, format!("html/{name}"));
        assert_eq!(screenshot, format!("screenshots/{name}.png"));
        assert_eq!(
            fs::read(output.join(html)).unwrap(),
            fs::read(wpt.join(name)).unwrap()
        );
        assert!(
            fs::read(output.join(screenshot))
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
    }
}

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

#[test]
fn tests_list_renders_exactly_the_listed_files() {
    let root = tempdir().unwrap();
    let wpt = root.path().join("wpt");
    let dir = wpt.join("css/CSS2");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("listed.xht"),
        r#"<html><head><link rel="match" href="ref.xht"/></head><body><p>Test passes if green.</p></body></html>"#,
    )
    .unwrap();
    fs::write(
        dir.join("unlisted.html"),
        "<meta name=assert content=x><body></body>",
    )
    .unwrap();
    let list = root.path().join("tests.txt");
    fs::write(&list, "css/CSS2/listed.xht\n").unwrap();
    let output = root.path().join("review");

    Command::cargo_bin("prepare-meta-assert-review")
        .unwrap()
        .arg("--wpt-root")
        .arg(&wpt)
        .arg("--tests")
        .arg(&list)
        .arg("--output")
        .arg(&output)
        .args(["--width", "8", "--height", "8"])
        .assert()
        .success();

    let manifest = fs::read_to_string(output.join("manifest.jsonl")).unwrap();
    let entries: Vec<serde_json::Value> = manifest
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["test_id"], "css/CSS2/listed.xht");
    assert_eq!(entries[0]["status"], "pending");
    assert!(
        fs::read(output.join("screenshots/css/CSS2/listed.xht.png"))
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );

    fs::write(&list, "css/CSS2/missing.xht\n").unwrap();
    Command::cargo_bin("prepare-meta-assert-review")
        .unwrap()
        .arg("--wpt-root")
        .arg(&wpt)
        .arg("--tests")
        .arg(&list)
        .arg("--output")
        .arg(&output)
        .assert()
        .failure();
    Command::cargo_bin("prepare-meta-assert-review")
        .unwrap()
        .arg("--wpt-root")
        .arg(&wpt)
        .arg("--tests")
        .arg(root.path().join("no-such-list.txt"))
        .arg("--output")
        .arg(&output)
        .assert()
        .failure();
}

#[test]
fn discovery_reads_exclude_baseline_and_records_render_errors() {
    let root = tempdir().unwrap();
    let wpt = root.path().join("wpt");
    let dir = wpt.join("css/foo");
    fs::create_dir_all(&dir).unwrap();
    for name in ["kept.html", "excluded.html"] {
        fs::write(
            dir.join(name),
            "<meta name=assert content='a box'><body></body>",
        )
        .unwrap();
    }
    let baseline = root.path().join("baseline.txt");
    fs::write(&baseline, "").unwrap();
    let exclude = root.path().join("exclude.txt");
    fs::write(&exclude, "css/foo/excluded.html\n").unwrap();
    let output = root.path().join("review");
    // A directory where the PNG should go makes the write fail, so the
    // test is recorded as a render error instead of aborting the run.
    fs::create_dir_all(output.join("screenshots/css/foo/kept.html.png")).unwrap();

    Command::cargo_bin("prepare-meta-assert-review")
        .unwrap()
        .arg("--wpt-root")
        .arg(&wpt)
        .arg("--baseline")
        .arg(&baseline)
        .arg("--exclude-baseline")
        .arg(&exclude)
        .arg("--output")
        .arg(&output)
        .args(["--width", "8", "--height", "8"])
        .assert()
        .success();

    let manifest = fs::read_to_string(output.join("manifest.jsonl")).unwrap();
    let entries: Vec<serde_json::Value> = manifest
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["test_id"], "css/foo/kept.html");
    assert_eq!(entries[0]["status"], "render-error");
    let template = fs::read_to_string(output.join("reviews.template.jsonl")).unwrap();
    assert!(template.is_empty());

    Command::cargo_bin("prepare-meta-assert-review")
        .unwrap()
        .arg("--wpt-root")
        .arg(root.path().join("no-such-wpt"))
        .arg("--baseline")
        .arg(&baseline)
        .arg("--output")
        .arg(&output)
        .assert()
        .failure();
}
