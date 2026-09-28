use super::*;
#[test]
fn output_flag_is_owned_once() {
    let mut args = vec![
        "runner".into(),
        "--results-json".into(),
        "result.json".into(),
        "--wpt-root".into(),
        "wpt".into(),
    ];
    assert_eq!(
        extract_output(&mut args).unwrap(),
        Some(std::path::PathBuf::from("result.json"))
    );
    assert_eq!(args, vec!["runner", "--wpt-root", "wpt"]);
    let mut args = vec!["--results-json".into()];
    assert!(extract_output(&mut args).is_err());
}

#[test]
fn output_flag_rejects_duplicate_paths_and_another_option_as_path() {
    let mut duplicate = vec![
        "--results-json".into(),
        "first.json".into(),
        "--results-json".into(),
        "second.json".into(),
    ];
    assert_eq!(
        extract_output(&mut duplicate).unwrap_err(),
        "duplicate --results-json"
    );
    let mut missing = vec!["--results-json".into(), "--wpt-root".into(), "wpt".into()];
    assert_eq!(
        extract_output(&mut missing).unwrap_err(),
        "--results-json requires a path"
    );
}

#[test]
fn absent_output_flag_preserves_arguments_and_needs_no_file() {
    let mut args = vec!["runner".into(), "--wpt-root".into(), "wpt".into()];
    let original = args.clone();
    assert_eq!(extract_output(&mut args).unwrap(), None);
    assert_eq!(args, original);
    write_results(None, &[]).unwrap();
}

#[test]
fn json_owns_ordered_duplicate_names_unicode_messages_and_page_errors() {
    let mut tests = vec![
        SubtestOutcome {
            name: "同名".into(),
            passed: true,
            message: String::new(),
        },
        SubtestOutcome {
            name: "同名".into(),
            passed: false,
            message: "FAIL: 行\n\"値\"".into(),
        },
    ];
    let records = vec![
        ResultRecord::new("pass.html", &tests, None),
        ResultRecord::new("error.html", &[], Some("aborted: too many tasks".into())),
    ];
    tests[0].name.clear();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("results.json");
    write_results(Some(&path), &records).unwrap();
    let actual: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        actual,
        serde_json::json!([
            {"test_id":"pass.html", "tests":[
                {"name":"同名", "passed":true, "message":""},
                {"name":"同名", "passed":false, "message":"FAIL: 行\n\"値\""}
            ], "error":null},
            {"test_id":"error.html", "tests":[], "error":"aborted: too many tasks"}
        ])
    );
}

#[test]
fn output_directory_reports_a_file_creation_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(write_results(Some(dir.path()), &[]).is_err());
    assert!(dir.path().is_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn full_device_reports_an_error_after_opening_the_output() {
    std::fs::File::create("/dev/full").unwrap();
    assert!(write_results(Some(std::path::Path::new("/dev/full")), &[]).is_err());
}
