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
