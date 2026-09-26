use super::*;

/// A stand-in for the real helper with the same `'use strict'` prologue and
/// the same `test_invalid_value`/`test_valid_value` test names.
const TESTCOMMON: &str = r#"'use strict';
function test_valid_value(property, value, serializedValue) {
    if (arguments.length < 3) serializedValue = value;
    test(function () {
        var div = document.getElementById('target') || document.createElement('div');
        div.style[property] = "";
        div.style[property] = value;
        assert_equals(div.style.getPropertyValue(property), serializedValue);
    }, "e.style['" + property + "'] = " + JSON.stringify(value) + " should set the property value");
}
function test_invalid_value(property, value) {
    test(function () {
        var div = document.getElementById('target') || document.createElement('div');
        div.style[property] = "";
        div.style[property] = value;
        assert_equals(div.style.getPropertyValue(property), "");
    }, "e.style['" + property + "'] = " + JSON.stringify(value) + " should not set the property value");
}
"#;

fn fixture(html: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("css/support")).unwrap();
    fs::create_dir_all(dir.path().join("css/some-cat/parsing")).unwrap();
    fs::write(
        dir.path().join("css/support/parsing-testcommon.js"),
        TESTCOMMON,
    )
    .unwrap();
    fs::write(dir.path().join("css/some-cat/parsing/t.html"), html).unwrap();
    dir
}

const FIXTURE_PATH: &str = "css/some-cat/parsing/t.html";

fn outcome(
    test_id: &str,
    results: &[(&str, bool)],
) -> Result<ParsingFileOutcome, ParsingFileError> {
    Ok(ParsingFileOutcome {
        test_id: test_id.to_owned(),
        outcomes: results
            .iter()
            .map(|(name, passed)| TestOutcome {
                name: (*name).to_owned(),
                passed: *passed,
                message: String::new(),
            })
            .collect(),
    })
}

const INVALID: &str = "e.style['x'] = \"y\" should not set the property value";
const VALID: &str = "e.style['x'] = \"y\" should set the property value";

#[test]
fn inline_scripts_skips_src_tags_and_keeps_bare_ones() {
    let html = r#"<script src="/resources/testharness.js"></script>
<script src="/css/support/parsing-testcommon.js"></script>
<script>
test_invalid_value("box-sizing", "margin-box");
</script>"#;
    let extracted = inline_scripts(html);
    assert!(extracted.contains("test_invalid_value(\"box-sizing\", \"margin-box\")"));
    assert!(!extracted.contains("testharness.js"));
}

#[test]
fn inline_scripts_concatenates_multiple_bare_tags() {
    let html = "<script>a();</script><script>b();</script>";
    let extracted = inline_scripts(html);
    assert!(extracted.contains("a();"));
    assert!(extracted.contains("b();"));
}

#[test]
fn inline_scripts_returns_empty_for_no_scripts() {
    assert_eq!(inline_scripts("<html><body>hi</body></html>"), "");
}

#[test]
fn inline_scripts_stops_at_an_unterminated_tag_or_body() {
    assert_eq!(inline_scripts("<script"), "");
    assert_eq!(inline_scripts("<script>never closed"), "");
}

#[test]
fn run_parsing_invalid_file_reports_no_invalid_value_calls() {
    // A file whose only assertion helper is test_valid_selector( (which
    // needs a CSSStyleSheet/CSSRule surface) has neither test_invalid_value(
    // nor test_valid_value(.
    let dir = fixture("<script>test_valid_selector(\"div\");</script>");
    let result = run_parsing_invalid_file(dir.path(), Path::new(FIXTURE_PATH));
    assert!(matches!(result, Err(ParsingFileError::NoInvalidValueCalls)));
}

#[test]
fn run_parsing_invalid_file_reports_a_missing_page_or_helper_as_io() {
    let dir = tempfile::tempdir().unwrap();
    let missing_page = run_parsing_invalid_file(dir.path(), Path::new(FIXTURE_PATH));
    assert!(matches!(missing_page, Err(ParsingFileError::Io(_))));

    let dir = fixture("<script>test_invalid_value(\"color\", \"bogus\");</script>");
    fs::remove_file(dir.path().join("css/support/parsing-testcommon.js")).unwrap();
    let missing_helper = run_parsing_invalid_file(dir.path(), Path::new(FIXTURE_PATH));
    let error = missing_helper.unwrap_err();
    assert!(matches!(error, ParsingFileError::Io(_)));
    assert!(error.to_string().starts_with("I/O error: "), "{error}");
}

#[test]
fn native_run_executes_the_helper_and_inline_script_on_a_live_document() {
    let dir = fixture(
        r#"<!DOCTYPE html>
<div id="target"></div>
<script src="/resources/testharness.js"></script>
<script src="/css/support/parsing-testcommon.js"></script>
<script>
undeclaredInSloppyMode = 1;
test_invalid_value("box-sizing", "margin-box");
test_valid_value("box-sizing", "border-box");
test(function () {
    assert_true(document.getElementById("target") instanceof HTMLElement);
}, "the page's own markup is live");
</script>"#,
    );
    let outcome = run_parsing_invalid_file(dir.path(), Path::new(FIXTURE_PATH)).unwrap();
    assert_eq!(outcome.test_id, FIXTURE_PATH);
    assert_eq!(outcome.total(), 3, "{:?}", outcome.outcomes);
    assert!(outcome.all_passed(), "{:?}", outcome.outcomes);
    assert_eq!(outcome.invalid_value_total(), 1);
    assert_eq!(outcome.invalid_value_passed(), 1);
}

#[test]
fn native_run_reports_a_valid_value_as_failing_the_invalid_check() {
    let dir = fixture("<script>test_invalid_value(\"box-sizing\", \"content-box\");</script>");
    let outcome = run_parsing_invalid_file(dir.path(), Path::new(FIXTURE_PATH)).unwrap();
    assert_eq!(outcome.total(), 1);
    assert!(!outcome.all_passed(), "{:?}", outcome.outcomes);
}

#[test]
fn native_run_reports_an_uncaught_script_error_as_a_harness_error() {
    let dir = fixture("<script>test_valid_value(\"color\", \"red\"); undefinedHelper();</script>");
    let error = run_parsing_invalid_file(dir.path(), Path::new(FIXTURE_PATH)).unwrap_err();
    assert!(matches!(error, ParsingFileError::Harness(_)), "{error:?}");
    assert!(error.to_string().contains("undefinedHelper"), "{error}");
}

#[test]
fn positive_control_passes_on_the_native_runtime() {
    let (host, ..) = live_host();
    let outcomes = run_testharness_scripts_on_host(
        &[
            POSITIVE_CONTROL_JS,
            "test(function () {}, 'after control');",
        ],
        host,
    )
    .unwrap();
    assert_eq!(outcomes.len(), 1);
}

/// An inert style binding — here a `getPropertyValue` that always reads back
/// empty, which would make every `test_invalid_value` pass — must fail the
/// positive control, turning the file into an error instead of a result.
#[test]
fn positive_control_catches_an_inert_style_binding() {
    let (host, ..) = live_host();
    let result = run_testharness_scripts_on_host(
        &[
            "CSSStyleDeclaration.prototype.getPropertyValue = function () { return ''; };",
            POSITIVE_CONTROL_JS,
            "test(function () {}, 'never trusted');",
        ],
        host,
    );
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("positive control failed"),
        "{error}"
    );
}

fn live_host() -> (WptDocumentHost, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        "<!DOCTYPE html><body></body>",
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    (WptDocumentHost::new(setup, dir.path()), dir)
}

#[test]
fn display_names_each_error_kind() {
    assert_eq!(
        ParsingFileError::Document("boom".into()).to_string(),
        "live document: boom"
    );
    assert_eq!(
        ParsingFileError::NoInvalidValueCalls.to_string(),
        "no test_invalid_value( calls in this file's inline script"
    );
}

#[test]
fn invalid_value_counts_ignore_other_test_names() {
    let result = outcome("a", &[(INVALID, true), (INVALID, false), (VALID, true)]).unwrap();
    assert_eq!(result.invalid_value_total(), 2);
    assert_eq!(result.invalid_value_passed(), 1);
    assert_eq!(result.total(), 3);
    assert_eq!(result.passed(), 2);
}

#[test]
fn compare_counts_only_invalid_value_outcomes() {
    let legacy_error: Result<ParsingFileOutcome, ParsingFileError> =
        Err(ParsingFileError::Io("gone".into()));
    let native_error: Result<ParsingFileOutcome, ParsingFileError> =
        Err(ParsingFileError::Io("gone".into()));
    let fewer_old = outcome("fewer", &[(INVALID, true), (INVALID, true)]);
    let fewer_new = outcome("fewer", &[(INVALID, true), (INVALID, false)]);
    let more_old = outcome("more", &[(INVALID, false)]);
    let more_new = outcome("more", &[(INVALID, true)]);
    // Extra non-invalid-value outcomes on native, passing or failing, do
    // not count either way.
    let same_old = outcome("same", &[(INVALID, true), (VALID, true)]);
    let same_new = outcome("same", &[(INVALID, true), (VALID, false), (VALID, false)]);
    let ok = outcome("x", &[(INVALID, true)]);

    let comparison = compare([
        ("regressed-to-error", &ok, &native_error),
        ("fixed-error", &legacy_error, &ok),
        ("both-error", &legacy_error, &native_error),
        ("fewer", &fewer_old, &fewer_new),
        ("more", &more_old, &more_new),
        ("same", &same_old, &same_new),
    ]);

    assert_eq!(
        comparison.regressions,
        vec![
            Difference {
                test_id: "regressed-to-error".into(),
                legacy: "1/1".into(),
                native: "error: I/O error: gone".into(),
            },
            Difference {
                test_id: "fewer".into(),
                legacy: "2/2".into(),
                native: "1/2".into(),
            },
        ]
    );
    assert_eq!(
        comparison.improvements,
        vec![
            Difference {
                test_id: "fixed-error".into(),
                legacy: "error: I/O error: gone".into(),
                native: "1/1".into(),
            },
            Difference {
                test_id: "more".into(),
                legacy: "0/1".into(),
                native: "1/1".into(),
            },
        ]
    );
}

#[test]
fn legacy_engine_still_runs_the_shim() {
    let dir = fixture("<script>test_invalid_value(\"box-sizing\", \"margin-box\");</script>");
    let outcome =
        run_parsing_invalid_file_with(dir.path(), Path::new(FIXTURE_PATH), Engine::Legacy).unwrap();
    assert_eq!(outcome.total(), 1);
    assert!(outcome.all_passed(), "{:?}", outcome.outcomes);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn run_parsing_invalid_file_on_real_fixture_is_seven_of_seven() {
    let wpt_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt");
    let outcome = run_parsing_invalid_file(
        &wpt_root,
        Path::new("css/css-sizing/parsing/box-sizing-invalid.html"),
    )
    .unwrap();
    assert_eq!(
        outcome.test_id,
        "css/css-sizing/parsing/box-sizing-invalid.html"
    );
    assert_eq!(outcome.total(), 7);
    assert!(outcome.all_passed(), "{:?}", outcome.outcomes);
}
