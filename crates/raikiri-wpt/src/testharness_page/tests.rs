use std::path::{Path, PathBuf};

use raikiri_js::runtime::{Abort, RunReport};

use super::*;

/// A stand-in for `resources/testharness.js` with the three entry points the
/// report script uses, plus `report(tests, status)`: a page calls it to have
/// `tests` and `status` delivered to every completion callback on `load`.
const FAKE_HARNESS: &str = r#"
var __callbacks = [];
function setup(options) { window.__setup_options = options; }
function add_completion_callback(callback) { __callbacks.push(callback); }
function report(tests, status) {
    window.addEventListener("load", function () {
        for (var i = 0; i < __callbacks.length; i++) {
            __callbacks[i](tests, status);
        }
    });
}
"#;

const HARNESS_TAGS: &str = "<script src=/resources/testharness.js></script>\
     <script src=/resources/testharnessreport.js></script>";

/// A WPT-root-like temp directory whose `resources/testharness.js` is
/// `harness`, with `body` (after the two harness `<script>` tags) written to
/// `css/t/page.html`.
fn page_root(harness: &str, body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("resources")).unwrap();
    std::fs::create_dir_all(dir.path().join("css/t")).unwrap();
    std::fs::write(dir.path().join("resources/testharness.js"), harness).unwrap();
    std::fs::write(
        dir.path().join("css/t/page.html"),
        format!("{HARNESS_TAGS}{body}"),
    )
    .unwrap();
    dir
}

fn run_fake(body: &str) -> Result<Vec<TestOutcome>, PageError> {
    let dir = page_root(FAKE_HARNESS, body);
    run_testharness_page(Path::new("css/t/page.html"), dir.path())
}

fn outcome(name: &str, passed: bool, message: &str) -> TestOutcome {
    TestOutcome {
        name: name.to_owned(),
        passed,
        message: message.to_owned(),
    }
}

#[test]
fn every_test_status_maps_onto_an_outcome() {
    let results = run_fake(
        "<script>report([\
            {name: 'pass', status: 0, message: null},\
            {name: 'fail', status: 1, message: 'boom'},\
            {name: 'timeout', status: 2, message: 'slow'},\
            {name: 'notrun', status: 3, message: undefined},\
            {name: 'precondition', status: 4, message: 'missing'},\
            {name: 'unknown', status: 9, message: 'odd'},\
            'not an object'\
        ], {status: 0, message: null});</script>",
    )
    .unwrap();
    assert_eq!(
        results,
        vec![
            outcome("pass", true, ""),
            outcome("fail", false, "FAIL: boom"),
            outcome("timeout", false, "TIMEOUT: slow"),
            outcome("notrun", false, "NOTRUN: "),
            outcome("precondition", false, "PRECONDITION_FAILED: missing"),
            outcome("unknown", false, "FAIL: odd"),
        ]
    );
}

#[test]
fn a_non_ok_harness_status_is_a_harness_error() {
    for (status, expected) in [
        (1, "ERROR: bad"),
        (2, "TIMEOUT: bad"),
        (3, "PRECONDITION_FAILED: bad"),
    ] {
        let result = run_fake(&format!(
            "<script>report([{{name: 'a', status: 0}}], {{status: {status}, message: 'bad'}});</script>"
        ));
        assert_eq!(result, Err(PageError::Harness(expected.to_owned())));
    }
}

#[test]
fn no_delivered_tests_is_no_results() {
    assert_eq!(
        run_fake("<script>report([], {status: 0});</script>"),
        Err(PageError::NoResults)
    );
    // Arguments that are not objects deliver nothing.
    assert_eq!(
        run_fake("<script>report(undefined, 'ok');</script>"),
        Err(PageError::NoResults)
    );
    // A page that never completes delivers nothing at all.
    assert_eq!(run_fake(""), Err(PageError::NoResults));
}

#[test]
fn only_the_first_delivery_counts() {
    let results = run_fake(
        "<script>add_completion_callback(function () {});\
         report([{name: 'first', status: 0}], {status: 0});\
         report([{name: 'second', status: 0}], {status: 0});</script>",
    )
    .unwrap();
    assert_eq!(results, vec![outcome("first", true, "")]);
}

#[test]
fn a_throwing_conversion_in_the_delivery_does_not_panic() {
    assert_eq!(
        run_fake("<script>report([{name: Symbol('x'), status: 0}], {status: 0});</script>"),
        Err(PageError::NoResults)
    );
}

#[test]
fn the_report_script_configures_the_harness_and_leaves_no_global_behind() {
    let results = run_fake(
        "<script>\
            var opts = window.__setup_options;\
            var leftover = Object.getOwnPropertySymbols(globalThis).filter(function (s) {\
                return s.description === 'raikiri testharness report sink';\
            }).length;\
            report([\
                {name: 'explicit_timeout', status: opts.explicit_timeout === true ? 0 : 1},\
                {name: 'output', status: opts.output === false ? 0 : 1},\
                {name: 'no symbol left', status: leftover === 0 ? 0 : 1},\
                {name: 'no name', status: typeof __raikiri_deliver === 'undefined' ? 0 : 1},\
            ], {status: 0});\
        </script>",
    )
    .unwrap();
    assert!(results.iter().all(|r| r.passed), "{results:?}");
    assert_eq!(results.len(), 4);
}

#[test]
fn report_script_looks_up_the_sink_by_its_symbol_description() {
    assert!(REPORT_SCRIPT.contains(&format!("\"{SINK_SYMBOL_DESCRIPTION}\"")));
}

#[test]
fn a_second_report_script_finds_no_sink_and_delivers_nowhere() {
    let results = run_fake(
        "<script src=/resources/testharnessreport.js></script>\
         <script>report([{name: 'a', status: 0}], {status: 0});</script>",
    )
    .unwrap();
    // The first report script took the sink off the global object; the
    // second one found none and registered a callback that drops the
    // results, so exactly one delivery is recorded.
    assert_eq!(results, vec![outcome("a", true, "")]);
}

#[test]
fn document_fonts_resolves_immediately() {
    let results = run_fake(
        "<script>\
            var loaded, ready;\
            document.fonts.load('10px x').then(function (list) { loaded = list; });\
            document.fonts.ready.then(function (fonts) { ready = fonts; });\
            window.addEventListener('DOMContentLoaded', function () {\
                report([\
                    {name: 'load', status: Array.isArray(loaded) && loaded.length === 0 ? 0 : 1},\
                    {name: 'ready', status: ready === document.fonts ? 0 : 1},\
                ], {status: 0});\
            });\
        </script>",
    )
    .unwrap();
    assert!(results.iter().all(|r| r.passed), "{results:?}");
}

#[test]
fn same_directory_and_root_relative_support_scripts_load() {
    let dir = page_root(
        FAKE_HARNESS,
        "<script src=local.js></script><script src=/css/support/shared.js></script>\
         <script>report([{name: 'local', status: local}, {name: 'shared', status: shared}], {status: 0});</script>",
    );
    std::fs::create_dir_all(dir.path().join("css/support")).unwrap();
    std::fs::write(dir.path().join("css/t/local.js"), "var local = 0;").unwrap();
    std::fs::write(dir.path().join("css/support/shared.js"), "var shared = 0;").unwrap();
    let results = run_testharness_page(Path::new("css/t/page.html"), dir.path()).unwrap();
    assert!(results.iter().all(|r| r.passed), "{results:?}");
}

#[test]
fn a_missing_page_is_a_host_error() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_testharness_page(Path::new("nope.html"), dir.path());
    assert!(
        matches!(&result, Err(PageError::Host(message)) if message.contains("nope.html")),
        "{result:?}"
    );
}

#[test]
fn a_harness_that_failed_to_load_is_a_host_error() {
    let dir = page_root(FAKE_HARNESS, "<script>test(function () {}, 'a');</script>");
    std::fs::remove_file(dir.path().join("resources/testharness.js")).unwrap();
    let result = run_testharness_page(Path::new("css/t/page.html"), dir.path());
    assert!(
        matches!(&result, Err(PageError::Host(message)) if message.contains("resources/testharness.js")),
        "{result:?}"
    );
}

#[test]
fn a_huge_delivered_length_is_capped() {
    let results = run_fake(
        "<script>report({length: Number.MAX_SAFE_INTEGER, 0: {name: 'a', status: 0}}, {status: 0});</script>",
    )
    .unwrap();
    assert_eq!(results, vec![outcome("a", true, "")]);
}

#[test]
fn a_resource_limit_abort_is_reported() {
    let result = run_fake("<script>function f() { f(); } f();</script>");
    assert_eq!(
        result,
        Err(PageError::Aborted(Abort::Recursion.to_string()))
    );
}

#[test]
fn abort_outranks_host_failures_which_outrank_the_harness() {
    let delivered = || Delivery {
        tests: vec![outcome("a", true, "")],
        harness_status: 1.0,
        harness_message: "bad".into(),
    };
    let mut report = RunReport {
        host_failures: vec!["layout".into(), "sheet".into()],
        ..RunReport::default()
    };
    assert_eq!(
        page_outcome(&report, Some(delivered())),
        Err(PageError::Host("layout; sheet".into()))
    );
    report.aborted = Some(Abort::Tasks);
    assert_eq!(
        page_outcome(&report, Some(delivered())),
        Err(PageError::Aborted(Abort::Tasks.to_string()))
    );
}

// The cases below run the real `resources/testharness.js` from the WPT
// checkout, copied into a temp root next to the page.

fn real_harness() -> String {
    let path: PathBuf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wpt/resources/testharness.js");
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn run_real(body: &str) -> Result<Vec<TestOutcome>, PageError> {
    let dir = page_root(&real_harness(), body);
    run_testharness_page(Path::new("css/t/page.html"), dir.path())
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn real_harness_sync_tests_pass_and_fail() {
    let results = run_real(
        "<script>\
            test(function () { assert_equals(1, 1); }, 'passes');\
            test(function () { assert_equals(1, 2, 'one is two'); }, 'fails');\
        </script>",
    )
    .unwrap();
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0], outcome("passes", true, ""));
    assert_eq!(results[1].name, "fails");
    assert!(!results[1].passed);
    assert!(
        results[1].message.starts_with("FAIL: ") && results[1].message.contains("one is two"),
        "{:?}",
        results[1]
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn real_harness_async_test_with_step_timeout_completes() {
    let results = run_real(
        "<script>\
            async_test(function (t) {\
                t.step_timeout(function () { t.done(); }, 100);\
            }, 'async');\
        </script>",
    )
    .unwrap();
    assert_eq!(results, vec![outcome("async", true, "")]);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn real_harness_promise_test_completes() {
    let results = run_real(
        "<script>\
            promise_test(function () {\
                return Promise.resolve().then(function (v) { assert_equals(v, undefined); });\
            }, 'promise');\
        </script>",
    )
    .unwrap();
    assert_eq!(results, vec![outcome("promise", true, "")]);
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn real_harness_uncaught_exception_is_a_harness_error() {
    let result = run_real(
        "<script>test(function () {}, 'fine');</script>\
         <script>throw new Error('outside any test');</script>",
    );
    assert!(
        matches!(&result, Err(PageError::Harness(message))
            if message.starts_with("ERROR: ") && message.contains("outside any test")),
        "{result:?}"
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn real_harness_without_tests_is_no_results() {
    assert_eq!(
        run_real("<p>nothing to test</p>"),
        Err(PageError::NoResults)
    );
}

#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn real_harness_loads_a_root_relative_support_script() {
    let dir = page_root(
        &real_harness(),
        "<script src=/css/support/helper.js></script>\
         <script>test(function () { assert_equals(helper(), 42); }, 'helper');</script>",
    );
    std::fs::create_dir_all(dir.path().join("css/support")).unwrap();
    std::fs::write(
        dir.path().join("css/support/helper.js"),
        "function helper() { return 42; }",
    )
    .unwrap();
    let results = run_testharness_page(Path::new("css/t/page.html"), dir.path()).unwrap();
    assert_eq!(results, vec![outcome("helper", true, "")]);
}

#[test]
fn page_errors_display_their_kind_and_detail() {
    assert_eq!(
        PageError::Harness("ERROR: boom".into()).to_string(),
        "harness: ERROR: boom"
    );
    assert_eq!(
        PageError::Aborted("too many tasks".into()).to_string(),
        "aborted: too many tasks"
    );
    assert_eq!(
        PageError::NoResults.to_string(),
        "the harness reported no test results"
    );
    assert_eq!(
        PageError::Host("missing.html: not found".into()).to_string(),
        "host: missing.html: not found"
    );
}
