//! Exercise strict expected-failure reporting through the compiled binary.

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn report_records_execution_errors_and_strict_mode_controls_exit_status() {
    let directory = tempdir().unwrap();
    let wpt_root = directory.path().join("wpt");
    let fonts = wpt_root.join("fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    let ahem = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../raikiri-dom/tests/data/text-autospace/Ahem.ttf");
    std::fs::copy(ahem, fonts.join("Ahem.ttf")).unwrap();
    let baseline = directory.path().join("baseline.txt");
    let report = directory.path().join("report.tsv");
    std::fs::write(&baseline, "css/missing/test.html\n").unwrap();

    let ordinary = Command::cargo_bin("run-baseline-report")
        .unwrap()
        .current_dir(directory.path())
        .args([
            "--wpt-root",
            wpt_root.to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--output",
            report.to_str().unwrap(),
        ])
        .assert()
        .success();
    let report_text = std::fs::read_to_string(&report).unwrap();
    assert!(
        report_text.starts_with("css/missing/test.html\tERROR\tdiscover:"),
        "expected a detailed execution error row, got {report_text:?}"
    );
    assert!(
        String::from_utf8_lossy(&ordinary.get_output().stderr).contains("ERROR 1"),
        "missing execution result count in stderr: {:?}",
        ordinary.get_output()
    );

    let strict = Command::cargo_bin("run-baseline-report")
        .unwrap()
        .current_dir(directory.path())
        .args([
            "--wpt-root",
            wpt_root.to_str().unwrap(),
            "--baseline",
            baseline.to_str().unwrap(),
            "--output",
            report.to_str().unwrap(),
            "--strict",
        ])
        .assert()
        .failure();
    assert!(
        String::from_utf8_lossy(&strict.get_output().stderr).contains("strict report found"),
        "strict failure reason not reported: {:?}",
        strict.get_output()
    );
}
