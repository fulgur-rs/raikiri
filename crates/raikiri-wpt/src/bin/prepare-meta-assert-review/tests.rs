use super::*;

#[test]
fn artifact_paths_preserve_normal_components_and_reject_parents() {
    assert_eq!(
        path_to_string(Path::new("css/名/test.html")).unwrap(),
        "css/名/test.html"
    );
    assert!(path_to_string(Path::new("../outside.html")).is_err());
    assert!(path_to_string(Path::new("css/../../outside.html")).is_err());
    assert!(path_to_string(Path::new("/outside.html")).is_err());
}

#[cfg(unix)]
#[test]
fn artifact_paths_preserve_backslashes_and_reject_lossy_names() {
    use std::os::unix::ffi::OsStrExt;
    assert_eq!(
        path_to_string(Path::new(r"css/a\b.html")).unwrap(),
        r"css/a\b.html"
    );
    let invalid = Path::new(std::ffi::OsStr::from_bytes(b"css/\xff.html"));
    assert!(path_to_string(invalid).is_err());
}

#[cfg(windows)]
#[test]
fn artifact_paths_use_portable_separators_and_reject_drive_roots() {
    assert_eq!(
        path_to_string(Path::new(r"css\foo\test.html")).unwrap(),
        "css/foo/test.html"
    );
    assert!(path_to_string(Path::new(r"C:\outside.html")).is_err());
}

#[test]
fn extracts_assert_with_attributes_in_either_order() {
    assert_eq!(
        extract_meta_assert(r#"<meta name='assert' content='one & two'>"#),
        Some("one & two".to_owned())
    );
    assert_eq!(
        extract_meta_assert(r#"<meta content="three" NAME="assert">"#),
        Some("three".to_owned())
    );
    assert_eq!(
        extract_meta_assert("<meta name='author' content='x'>"),
        None
    );
}

#[test]
fn identifies_only_exact_parsing_path_components() {
    assert!(has_path_component("css/foo/parsing/a.html", "parsing"));
    assert!(!has_path_component(
        "css/foo/parsing-extra/a.html",
        "parsing"
    ));
}

#[test]
fn path_prefix_matches_a_directory_without_matching_similar_names() {
    assert!(matches_path_prefix(
        "css/css-backgrounds/test.html",
        "css/css-backgrounds/"
    ));
    assert!(matches_path_prefix(
        "css/css-backgrounds",
        "css/css-backgrounds"
    ));
    assert!(!matches_path_prefix(
        "css/css-backgrounds-extra/test.html",
        "css/css-backgrounds"
    ));
}

#[test]
fn json_string_escapes_manifest_values() {
    assert_eq!(json_string("a\"b\\c\n"), r#""a\"b\\c\n""#);
}

#[test]
fn html_extension_filter_includes_wpt_variants() {
    assert!(is_html_like(Path::new("a.html")));
    assert!(is_html_like(Path::new("a.xht")));
    assert!(!is_html_like(Path::new("a.js")));
}

#[test]
fn listed_candidates_keep_reftests_and_tests_without_assert() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("css/CSS2");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("with-ref.xht"),
        r#"<html><head><link rel="match" href="ref.xht"/><meta name="assert" content="A"/></head></html>"#,
    )
    .unwrap();
    fs::write(dir.join("plain.html"), "<p>Test passes if green.").unwrap();
    let list = root.path().join("list.txt");
    fs::write(
        &list,
        "# comment\ncss/CSS2/with-ref.xht\n\ncss/CSS2/plain.html\n",
    )
    .unwrap();

    let candidates = listed_candidates(root.path(), &list).unwrap();
    let ids: Vec<_> = candidates.iter().map(|c| c.test_id.as_str()).collect();
    assert_eq!(ids, ["css/CSS2/with-ref.xht", "css/CSS2/plain.html"]);
    assert_eq!(candidates[0].assert_text, "A");
    assert_eq!(candidates[1].assert_text, "");

    fs::write(&list, "../outside.html\n").unwrap();
    assert!(listed_candidates(root.path(), &list).is_err());
}

#[test]
fn tests_option_takes_a_list_path() {
    let args = parse_args(&["--tests".to_owned(), "list.txt".to_owned()]).unwrap();
    assert_eq!(args.tests, Some(PathBuf::from("list.txt")));
}

#[test]
fn render_png_writes_the_first_page_and_reports_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let test = dir.path().join("test.html");
    fs::write(
        &test,
        "<html><body style=\"margin:0;background:blue\"></body></html>",
    )
    .unwrap();
    let output = dir.path().join("out.png");

    render_png(&test, &output, 20, 10).unwrap();
    let decoder = png::Decoder::new(io::BufReader::new(fs::File::open(&output).unwrap()));
    let info = decoder.read_info().unwrap().info().clone();
    assert_eq!((info.width, info.height), (20, 10));

    assert!(render_png(&dir.path().join("missing.html"), &output, 20, 10).is_err());
}
