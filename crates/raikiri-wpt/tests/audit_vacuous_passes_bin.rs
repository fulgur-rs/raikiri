//! Exercise the vacuous-PASS audit through the compiled binary.

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn audit_writes_one_verdict_per_id_and_rejects_bad_arguments() {
    let directory = tempdir().unwrap();
    let wpt_root = directory.path().join("wpt");
    let fonts = wpt_root.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    let ahem = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../raikiri-dom/tests/data/text-autospace/Ahem.ttf");
    std::fs::copy(ahem, fonts.join("Ahem.ttf")).unwrap();
    let tests = wpt_root.join("css/example");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(
        tests.join("blank.html"),
        r#"<!DOCTYPE html><link rel="match" href="ref.html"><div style="color:green"></div>"#,
    )
    .unwrap();
    std::fs::write(tests.join("ref.html"), "<!DOCTYPE html><p></p>").unwrap();
    let ids = directory.path().join("ids.txt");
    std::fs::write(
        &ids,
        "# comment\ncss/example/blank.html\ncss/example/missing.html\n",
    )
    .unwrap();
    let report = directory.path().join("out/audit.tsv");

    let run = Command::cargo_bin("audit-vacuous-passes")
        .unwrap()
        .current_dir(directory.path())
        .args([
            "--wpt-root",
            wpt_root.to_str().unwrap(),
            "--ids",
            ids.to_str().unwrap(),
            "--output",
            report.to_str().unwrap(),
            "--jobs",
            "2",
        ])
        .assert()
        .success();
    let text = std::fs::read_to_string(&report).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text:?}");
    assert!(lines[0].starts_with("css/example/blank.html\tVACUOUS-BLANK\t"));
    assert!(lines[1].starts_with("css/example/missing.html\tNOT-PASS\tERROR: "));
    let stderr = String::from_utf8_lossy(&run.get_output().stderr).into_owned();
    assert!(
        stderr.contains("-- NOT-PASS 1, VACUOUS-BLANK 1 (2 total) --"),
        "{stderr}"
    );

    let stdout = Command::cargo_bin("audit-vacuous-passes")
        .unwrap()
        .current_dir(directory.path())
        .args([
            "--wpt-root",
            wpt_root.to_str().unwrap(),
            "--ids",
            ids.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert_eq!(String::from_utf8_lossy(&stdout.get_output().stdout), text);

    Command::cargo_bin("audit-vacuous-passes")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
    for args in [
        &["--jobs", "0", "--ids", "x"][..],
        &["--bogus"][..],
        &["--ids"][..],
        &[][..],
        &["--ids", "missing-file.txt"][..],
    ] {
        Command::cargo_bin("audit-vacuous-passes")
            .unwrap()
            .current_dir(directory.path())
            .args(args)
            .assert()
            .failure();
    }
}
