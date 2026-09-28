use std::path::{Path, PathBuf};

use raikiri_js::runtime::{Abort, RunReport};
use raikiri_js_wasmtime_harness::SINK_SYMBOL_DESCRIPTION;

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

fn run_fake(body: &str) -> Result<Vec<SubtestOutcome>, PageError> {
    let dir = page_root(FAKE_HARNESS, body);
    run_testharness_page(Path::new("css/t/page.html"), dir.path())
}

#[test]
fn nonfinite_harness_status_is_a_harness_error() {
    for status in ["NaN", "Infinity", "-Infinity"] {
        let result = run_fake(&format!(
            "<script>report([{{name:'done',status:0,message:''}}],{{status:{status},message:'bad status'}});</script>"
        ));
        assert_eq!(result, Err(PageError::Harness("ERROR: bad status".into())));
    }
}

#[test]
fn clean_abort_outranks_delivered_nonfinite_harness_status() {
    let result = run_fake(
        "<script>window.addEventListener('load', function(){\
         __callbacks[0]([{name:'done',status:0,message:''}],{status:NaN,message:'bad status'});\
         (function f(){f();})();\
         });</script>",
    );
    assert_eq!(
        result,
        Err(PageError::Aborted(Abort::Recursion.to_string()))
    );
}

#[test]
fn clean_abort_outranks_oversized_final_document_transfer() {
    let result = run_fake(
        "<script>document.body.appendChild(document.createTextNode('\\0'.repeat(3000000)));\
         setTimeout(function(){},40000);</script>",
    );
    assert_eq!(
        result,
        Err(PageError::Aborted(Abort::VirtualTime.to_string()))
    );
}

#[test]
fn clean_abort_outranks_oversized_result_transfer() {
    let result = run_fake(
        "<script>report([{name:'\\0'.repeat(3000000),status:0,message:''}],{status:0});\
         window.addEventListener('load',function(){(function f(){f();})();});</script>",
    );
    assert_eq!(
        result,
        Err(PageError::Aborted(Abort::Recursion.to_string()))
    );
}

#[cfg(feature = "js-wasmtime")]
#[test]
fn oversized_final_document_without_abort_remains_a_host_error() {
    let result = run_fake(
        "<script>document.body.appendChild(document.createTextNode('\\0'.repeat(3000000)));\
         report([{name:'done',status:0,message:''}],{status:0});</script>",
    );
    assert!(matches!(result, Err(PageError::Host(message)) if message.contains("transport cap")));
}

fn outcome(name: &str, passed: bool, message: &str) -> SubtestOutcome {
    SubtestOutcome {
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

/// `explicit_timeout: true` (see [`REPORT_SCRIPT`]) disables a real
/// testharness.js's own self-timeout, so an `async_test` that never calls
/// `t.done()` never delivers anything on its own; only a call to the
/// harness's `timeout()` can still force such a run to finish.
/// [`FAKE_HARNESS`] does not model any of that real timing machinery -- it
/// has no notion of `test`/`async_test`/`timeout` at all -- so this builds a
/// page with its own minimal stand-ins for the three, defined directly in the
/// page's script: a `test()` that runs its function synchronously and
/// records PASS/FAIL, an `async_test()` that records a test but
/// (deliberately) never completes it, and a `timeout()` -- its body supplied
/// by the caller, so each test below can make it misbehave in a different
/// way -- that would otherwise deliver whatever was recorded so far.
fn never_completing_page(timeout_body: &str) -> String {
    format!(
        "<script>\
            var __results = [];\
            function test(fn, name) {{\
                var t = {{ name: name, status: 0, message: null }};\
                try {{ fn(); }} catch (e) {{ t.status = 1; t.message = String(e); }}\
                __results.push(t);\
            }}\
            function async_test(fn, name) {{\
                var t = {{ name: name, status: 3, message: null }};\
                __results.push(t);\
                fn(t);\
                return t;\
            }}\
            function timeout() {{ {timeout_body} }}\
            test(function () {{}}, 'sync passes');\
            async_test(function (t) {{ /* never calls t.done() */ }}, 'async never done');\
        </script>"
    )
}

/// The real harness's `tests.timeout()` completes without ever setting a
/// harness message when no test is mid-cleanup (see
/// `Tests.prototype.timeout` in testharness.js); `deliver` then reads that
/// missing message back as `""` (see [`string_property`]), producing
/// `"TIMEOUT: "` with nothing after the colon. This delivery mirrors that
/// exactly, so the fake and real harness tests below agree on the message.
const TIMEOUT_DELIVERS_RECORDED_RESULTS: &str = "for (var i = 0; i < __callbacks.length; i++) { \
     __callbacks[i](__results, { status: 2, message: null }); \
 }";

#[test]
fn an_async_test_that_never_completes_is_a_timeout_not_no_results() {
    let result = run_fake(&never_completing_page(TIMEOUT_DELIVERS_RECORDED_RESULTS));
    assert_eq!(result, Err(PageError::Harness("TIMEOUT: ".into())));
}

/// A forced timeout that still finds nothing to report (no test was ever
/// registered) is the one case [`finish_page`] deliberately does not treat
/// as a timeout: it is the same "nothing to report" page
/// [`no_delivered_tests_is_no_results`] above already covers, just reached
/// through the probe instead of a normal delivery.
#[test]
fn a_forced_timeout_with_no_tests_is_still_no_results() {
    let result = run_fake(
        "<script>\
            function timeout() {\
                for (var i = 0; i < __callbacks.length; i++) {\
                    __callbacks[i]([], { status: 2, message: 'Test timed out' });\
                }\
            }\
        </script>",
    );
    assert_eq!(result, Err(PageError::NoResults));
}

/// [`run_testharness_page`]'s own doc comment documents a failed script
/// fetch as outranking a non-OK harness status; [`finish_page`] must not let
/// the timeout probe undo that by manufacturing a harness status of its own
/// once a fetch has already failed, even though the page still has a
/// callable `timeout()` (from testharness.js loading successfully) and never
/// otherwise delivers.
#[test]
fn a_failed_script_fetch_outranks_a_forced_timeout() {
    let result = run_fake(
        "<script src=missing.js></script>\
         <script>\
            function timeout() {\
                for (var i = 0; i < __callbacks.length; i++) {\
                    __callbacks[i]([{ name: 'a', status: 3 }], { status: 2, message: null });\
                }\
            }\
        </script>",
    );
    assert!(
        matches!(&result, Err(PageError::Host(message)) if message.contains("missing.js")),
        "{result:?}"
    );
}

#[test]
fn a_throwing_timeout_probe_is_reported_as_a_host_failure() {
    let result = run_fake(
        "<script>\
            function timeout() { throw new Error('probe boom'); }\
        </script>",
    );
    assert!(
        matches!(&result, Err(PageError::Host(message))
            if message.contains("timeout probe") && message.contains("probe boom")),
        "{result:?}"
    );
}

#[test]
fn a_recursive_timeout_probe_is_reported_as_aborted() {
    let result = run_fake(
        "<script>\
            function timeout() { (function f() { f(); })(); }\
        </script>",
    );
    assert_eq!(
        result,
        Err(PageError::Aborted(Abort::Recursion.to_string()))
    );
}

/// The probe's own `evaluate()` call succeeds here (scheduling a timer
/// throws nothing), so this exercises the *second* `run_until_idle()` call
/// in [`probe_timeout`] hitting a limit, not the first one -- a task due
/// well past `Limits::default()`'s `max_virtual_time_ms` aborts the instant
/// the drain looks at it, with nothing else queued that could hit a
/// different limit first.
#[test]
fn a_timeout_probe_that_schedules_a_far_future_task_is_reported_as_aborted() {
    let result = run_fake(
        "<script>\
            function timeout() { setTimeout(function () {}, 40000); }\
        </script>",
    );
    assert_eq!(
        result,
        Err(PageError::Aborted(Abort::VirtualTime.to_string()))
    );
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

fn run_real(body: &str) -> Result<Vec<SubtestOutcome>, PageError> {
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

/// The real counterpart to
/// [`an_async_test_that_never_completes_is_a_timeout_not_no_results`]: a
/// passing synchronous `test()` alongside an `async_test` that registers
/// itself but never calls `t.done()` never reaches `add_completion_callback`
/// on its own (`explicit_timeout: true` disables testharness.js's own
/// self-timeout), so without [`probe_timeout`]'s `timeout()` call this page
/// would run to the end of `run_document()`'s draining and come back
/// [`PageError::NoResults`] -- indistinguishable from a page with no tests at
/// all.
#[test]
#[ignore = "requires the sparse WPT checkout from scripts/wpt/fetch.sh"]
fn real_harness_async_test_that_never_completes_times_out() {
    let result = run_real(
        "<script>\
            test(function () {}, 'sync passes');\
            async_test(function (t) { /* never calls t.done() */ }, 'async never done');\
        </script>",
    );
    assert_eq!(result, Err(PageError::Harness("TIMEOUT: ".into())));
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
    assert_eq!(
        PageError::Preamble("Error: bad".into()).to_string(),
        "preamble: Error: bad"
    );
}

#[test]
fn a_preamble_runs_before_every_page_script() {
    let dir = page_root(
        FAKE_HARNESS,
        "<script>report([{name: window.__order.join(','), status: 0}], {status: 0});</script>",
    );
    let results = run_testharness_page_with_preamble(
        Path::new("css/t/page.html"),
        dir.path(),
        "window.__order = ['preamble', typeof add_completion_callback];",
    )
    .unwrap();
    assert_eq!(results, vec![outcome("preamble,undefined", true, "")]);
}

#[test]
fn a_throwing_preamble_stops_the_page_as_a_preamble_error() {
    let dir = page_root(
        FAKE_HARNESS,
        "<script>report([{name: 'never', status: 0}], {status: 0});</script>",
    );
    let result = run_testharness_page_with_preamble(
        Path::new("css/t/page.html"),
        dir.path(),
        "throw new Error('sanity check failed');",
    );
    let Err(PageError::Preamble(message)) = result else {
        panic!("expected a preamble error, got {result:?}");
    };
    assert!(message.contains("sanity check failed"), "{message}");
    // A page that cannot be set up is still a host error, preamble or not.
    let missing = run_testharness_page_with_preamble(Path::new("missing.html"), dir.path(), "");
    assert!(matches!(missing, Err(PageError::Host(_))), "{missing:?}");
}

#[test]
fn repeated_flush_preserves_detached_template_and_namespaced_identity() {
    let outcomes = run_fake(r#"<script>
        try {
        var box = document.createElement('div');
        box.style.width = '37px';
        document.body.appendChild(box);
        var detached = document.createElement('span');
        detached.textContent = 'detached';
        var template = document.createElement('template');
        template.innerHTML = '<span>template text</span>';
        document.body.appendChild(template);
        var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svg.setAttribute('value', 'kept');
        var sheet = document.createElement('style');
        sheet.textContent = 'div::before { content: "derived"; }';
        document.head.appendChild(sheet);
        var width1 = box.getBoundingClientRect().width;
        box.style.color = 'red';
        var width2 = box.getBoundingClientRect().width;
        report([
          {name:'repeat geometry',status:width1 === width2 && width2 === 37 ? 0 : 1},
          {name:'detached identity',status:detached.textContent === 'detached' && detached.parentNode === null ? 0 : 1},
          {name:'template identity',status:template.innerHTML === '<span>template text</span>' && template.childNodes.length === 0 ? 0 : 1},
          {name:'element namespace and attributes',status:svg.namespaceURI === 'http://www.w3.org/2000/svg' && svg.getAttribute('value') === 'kept' ? 0 : 1}
        ],{status:0});
        } catch (error) { report([{name:"setup failure",status:1,message:String(error)}],{status:0}); }
    </script>"#).unwrap();
    assert!(outcomes.iter().all(|t| t.passed), "{outcomes:?}");
    assert_eq!(outcomes.len(), 4);
}

#[cfg(feature = "js-wasmtime")]
#[test]
fn native_flush_snapshot_preserves_template_and_namespaced_metadata() {
    use raikiri_js::runtime::DocumentHost;
    let dir = page_root(
        FAKE_HARNESS,
        r#"<style>div::before {content:'generated'}</style><div>text</div><template><span>inert</span></template><svg xmlns:x="urn:test" x:attr="kept"></svg>"#,
    );
    let mut host = prepare_host(Path::new("css/t/page.html"), dir.path()).unwrap();
    host.document_mut()
        .create_detached_element("detached")
        .unwrap();
    let before = host.document().logical_snapshot();
    host.flush().unwrap();
    assert_eq!(host.document().logical_snapshot(), before);
    *host.document_mut() =
        raikiri_dom::Document::from_logical_snapshot(before.clone(), 1000).unwrap();
    host.flush().unwrap();
    assert_eq!(host.document().logical_snapshot(), before);
}
