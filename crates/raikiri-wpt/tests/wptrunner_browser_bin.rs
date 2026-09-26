//! Process exit status and diagnostics for the native WPT browser binary.

use assert_cmd::Command;

#[test]
fn missing_arguments_return_failure_and_a_diagnostic() {
    for arguments in [vec![], vec!["print-reftest"]] {
        let assertion = Command::cargo_bin("raikiri-wpt-browser")
            .unwrap()
            .args(arguments)
            .assert()
            .code(1);
        let error = String::from_utf8_lossy(&assertion.get_output().stderr);
        assert!(error.contains("missing required argument --url"));
        assert!(assertion.get_output().stdout.is_empty());
    }
}
