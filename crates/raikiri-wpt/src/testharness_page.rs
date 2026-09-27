//! Run one WPT testharness page end to end with the real, unmodified
//! `resources/testharness.js` from the checkout: the page's own `<script>`
//! elements run in document order through [`DomRuntime::run_document`], and
//! this crate's report script (served in place of
//! `resources/testharnessreport.js`, see [`REPORT_SCRIPT`]) hands the
//! harness's completion results back to Rust.

use std::path::Path;

use boa_engine::object::FunctionObjectBuilder;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{Context, JsResult, JsString, JsSymbol, JsValue, NativeFunction, js_string};
use raikiri_js::TestOutcome;
use raikiri_js::runtime::{DomRuntime, RunReport, RuntimeError};

use crate::reftest::{DEFAULT_REFTTEST_HEIGHT, DEFAULT_REFTTEST_WIDTH, prepare_wpt_live_document};
use crate::wpt_host::WptDocumentHost;

/// Description of the symbol under which the result sink is handed to
/// [`REPORT_SCRIPT`]; the report script looks the symbol up by this
/// description, so the two must match.
const SINK_SYMBOL_DESCRIPTION: &str = "raikiri testharness report sink";

/// Served for `/resources/testharnessreport.js`. It configures the harness
/// (no timeout of its own, no DOM output) and registers a completion callback
/// that passes the harness's `tests` and `harness_status` to the result sink.
///
/// The sink is a native function that [`run_testharness_page`] installs on
/// the global object under a fresh, non-enumerable symbol key right before the
/// page runs. The first statement of this script takes it off the global
/// object again (deleting the property) and passes it as the argument of the
/// function that wraps the rest of the script, so from then on only that
/// closure can reach it: no global name ever refers to the sink.
pub(crate) const REPORT_SCRIPT: &str = r#"(function (__raikiri_deliver) {
    setup({ explicit_timeout: true, output: false });
    add_completion_callback(function (tests, harness_status) {
        __raikiri_deliver(tests, harness_status);
    });
})((function () {
    var symbols = Object.getOwnPropertySymbols(globalThis);
    for (var i = 0; i < symbols.length; i++) {
        if (symbols[i].description === "raikiri testharness report sink") {
            var sink = globalThis[symbols[i]];
            delete globalThis[symbols[i]];
            return sink;
        }
    }
    return function () {};
})());
"#;

/// Stand-in for `document.fonts` (CSS Font Loading): `load()` resolves to an
/// empty list and `ready` to the set itself. Fonts are registered before
/// layout, so there is nothing to wait for and no effect on layout.
const DOCUMENT_FONTS_SCRIPT: &str = r#"(function () {
    var fonts = {
        load: function () { return Promise.resolve([]); }
    };
    fonts.ready = Promise.resolve(fonts);
    document.fonts = fonts;
})();
"#;

/// Why a page produced no trustworthy list of subtest results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageError {
    /// The harness itself finished with a status other than OK
    /// (`"ERROR: ..."`, `"TIMEOUT: ..."`, `"PRECONDITION_FAILED: ..."`), for
    /// example because of an uncaught exception outside any test.
    Harness(String),
    /// A resource limit stopped the page before it finished.
    Aborted(String),
    /// The harness completed without reporting a single test, or never
    /// completed while every script loaded and registered no test of its own
    /// for a forced `timeout()` (see [`probe_timeout`]) to report either.
    NoResults,
    /// The page could not be read or set up, the embedder failed while
    /// scripts ran (layout, stylesheet, fragment parsing), or the harness
    /// never completed and some script failed to load.
    Host(String),
    /// The script run ahead of the page's own scripts threw, so the page
    /// was not run.
    Preamble(String),
}

impl std::fmt::Display for PageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Harness(message) => write!(f, "harness: {message}"),
            Self::Aborted(reason) => write!(f, "aborted: {reason}"),
            Self::NoResults => write!(f, "the harness reported no test results"),
            Self::Host(message) => write!(f, "host: {message}"),
            Self::Preamble(message) => write!(f, "preamble: {message}"),
        }
    }
}

impl std::error::Error for PageError {}

/// The most subtests read from one delivery. The loop over the delivered
/// list runs natively, outside the script engine's own loop limit, so a
/// page-controlled `length` must not decide how long it runs.
const MAX_DELIVERED_TESTS: u64 = 100_000;

/// What the harness handed to the result sink: the per-test results, and
/// the harness status and its message.
#[derive(Debug, Default)]
struct Delivery {
    tests: Vec<TestOutcome>,
    harness_status: f64,
    harness_message: String,
}

/// Run one WPT testharness page with the real testharness.js through
/// `run_document`. `path` is the page's file, relative to `wpt_root` (an
/// absolute path works too).
///
/// When several things went wrong at once the most fundamental one is
/// reported, since it makes everything after it untrustworthy: a resource
/// limit abort, then a host failure, then (when the harness never
/// completed) a script that failed to load, then a non-OK harness status,
/// then an empty result list.
pub(crate) fn run_testharness_page(
    path: &Path,
    wpt_root: &Path,
) -> Result<Vec<TestOutcome>, PageError> {
    let mut runtime = prepare_page(path, wpt_root)?;
    finish_page(&mut runtime)
}

/// Like [`run_testharness_page`], but first evaluates `preamble` as a
/// classic script of its own, after the page's document is built and before
/// any of the page's `<script>` elements run. An uncaught error in it stops
/// the page there, reported as [`PageError::Preamble`], so a failed sanity
/// check never mixes with the page's own results.
pub(crate) fn run_testharness_page_with_preamble(
    path: &Path,
    wpt_root: &Path,
    preamble: &str,
) -> Result<Vec<TestOutcome>, PageError> {
    let mut runtime = prepare_page(path, wpt_root)?;
    runtime
        .evaluate(preamble)
        .map_err(|error| PageError::Preamble(error.to_string()))?;
    finish_page(&mut runtime)
}

/// Read the page, build its live document and a runtime over it, and install
/// the page support scripts; no page script has run yet.
fn prepare_page(path: &Path, wpt_root: &Path) -> Result<DomRuntime, PageError> {
    let page = wpt_root.join(path);
    let html = std::fs::read_to_string(&page)
        .map_err(|error| PageError::Host(format!("{}: {error}", page.display())))?;
    let page_dir = page.parent().unwrap_or(wpt_root);
    let setup = prepare_wpt_live_document(
        &html,
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        page_dir,
        wpt_root,
    )
    .map_err(PageError::Host)?; // cov:ignore: the page was read as valid UTF-8 and HTML parsing recovers from markup errors.
    let mut host = WptDocumentHost::new(setup, wpt_root);
    if let Some(url) = std::fs::canonicalize(&page)
        .ok()
        .and_then(|page| raikiri::Url::from_file_path(page).ok())
    {
        host = host.with_page_url(url);
    }
    let mut runtime = DomRuntime::new(host).map_err(|error| PageError::Host(error.to_string()))?; // cov:ignore: building a realm over a parsed document does not fail.
    let installed = install_page_support(&mut runtime);
    installed.map_err(|error| PageError::Host(error.to_string()))?; // cov:ignore: only defines properties on a fresh realm's own objects.
    Ok(runtime)
}

/// Run the page's scripts and turn the run and whatever the harness
/// delivered into the page's result.
///
/// A page can run to completion (no abort, no host failure, no failed script
/// fetch) without the harness ever delivering anything:
/// `setup({explicit_timeout: true, ...})` (see [`REPORT_SCRIPT`]) disables
/// testharness.js's own self-timeout, so an `async_test`/`promise_test` that
/// never finishes waiting leaves nothing to read back, indistinguishable on
/// its own from a page with zero tests. In that case only, [`probe_timeout`]
/// calls the harness's `timeout()` -- testharness.js's own counterpart to
/// `explicit_timeout`, the only thing that can still force such a run to
/// finish -- and gives the event loop one more turn to let the synchronous
/// completion callback it triggers run.
///
/// A failed script fetch already outranks a harness status in
/// [`run_testharness_page`]'s own documented precedence (it explains an
/// incomplete run better than a harness status can, since the harness itself
/// may never even have found out why it was left incomplete), so the probe
/// does not run at all once `report.fetch_errors` is non-empty: forcing a
/// timeout there would let a manufactured `Harness("TIMEOUT: ...")` hide a
/// more specific, already-known reason instead of leaving it to
/// [`page_outcome`]'s existing fetch-error check.
///
/// A forced timeout that still reports zero tests is discarded rather than
/// kept: that is the page-with-nothing-registered case the probe exists to
/// leave alone (see [`probe_timeout`]'s own doc comment), so it falls through
/// to [`PageError::NoResults`] as if the probe had never run.
fn finish_page(runtime: &mut DomRuntime) -> Result<Vec<TestOutcome>, PageError> {
    let mut report = runtime.run_document();
    let mut delivery = take_delivery(runtime);
    if delivery.is_none()
        && report.aborted.is_none()
        && report.host_failures.is_empty()
        && report.fetch_errors.is_empty()
    {
        delivery =
            probe_timeout(runtime, &mut report).filter(|delivery| !delivery.tests.is_empty());
    }
    page_outcome(&report, delivery)
}

/// The harness's delivery, if it has made one yet.
fn take_delivery(runtime: &mut DomRuntime) -> Option<Delivery> {
    runtime
        .context_mut()
        .remove_data::<Delivery>()
        .map(|delivery| *delivery)
}

/// Called only when the page ran to completion but the harness never
/// delivered anything, with no host failure or failed script fetch already
/// explaining why (see [`finish_page`]'s own doc comment for both). Evaluates
/// a `typeof` probe rather than calling `timeout()` directly, since a page
/// that never loaded testharness.js at all has no `timeout` global -- that
/// case must stay [`PageError::NoResults`], not a manufactured timeout. A
/// limit hit while running the probe or draining its aftermath is folded into
/// `report` the same way an earlier abort would be, so [`page_outcome`]'s
/// existing abort precedence covers it without a second code path; an
/// ordinary exception from the probe itself is folded into
/// `report.host_failures` for the same reason -- surfaced, never silently
/// dropped, but without a new [`PageError`] variant just for it.
fn probe_timeout(runtime: &mut DomRuntime, report: &mut RunReport) -> Option<Delivery> {
    match runtime.evaluate("if (typeof timeout === 'function') { timeout(); }") {
        Ok(_) => {}
        Err(RuntimeError::Aborted(reason)) => {
            report.aborted = Some(reason);
            return None;
        }
        Err(RuntimeError::JavaScript(message) | RuntimeError::Host(message)) => {
            report
                .host_failures
                .push(format!("timeout probe: {message}"));
            return None;
        }
    }
    if let Err(reason) = runtime.run_until_idle() {
        report.aborted = Some(reason);
        return None;
    }
    take_delivery(runtime)
}

/// Install `document.fonts` and the result sink before any page script runs.
fn install_page_support(runtime: &mut DomRuntime) -> Result<(), raikiri_js::runtime::RuntimeError> {
    runtime.evaluate(DOCUMENT_FONTS_SCRIPT)?;
    install_result_sink(runtime.context_mut())
        .map_err(|error| raikiri_js::runtime::RuntimeError::JavaScript(error.to_string())) // cov:ignore: defining a fresh symbol-keyed property on the global object does not fail.
}

/// Put the native result sink on the global object under a new symbol whose
/// description [`REPORT_SCRIPT`] looks for: non-enumerable and configurable,
/// so the report script can delete it once it holds the function.
fn install_result_sink(context: &mut Context) -> JsResult<()> {
    let sink = FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(deliver))
        .name(js_string!("deliver"))
        .length(2)
        .build();
    let key = JsSymbol::new(Some(JsString::from(SINK_SYMBOL_DESCRIPTION)));
    let key =
        key.ok_or_else(|| boa_engine::JsNativeError::range().with_message("out of symbols"))?; // cov:ignore: symbol ids run out only after 2^64 symbols.
    let descriptor = PropertyDescriptor::builder()
        .value(sink)
        .writable(false)
        .enumerable(false)
        .configurable(true);
    context
        .global_object()
        .define_property_or_throw(key, descriptor, context)?;
    Ok(())
}

/// The result sink: `(tests, harness_status)` from the harness's completion
/// callback. Records the first delivery in the context's data; a later one
/// is ignored. Reading the arguments runs ordinary property lookups and
/// conversions, so a hostile value can only throw, never panic.
fn deliver(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if context.get_data::<Delivery>().is_some() {
        return Ok(JsValue::undefined());
    }
    let tests_value = args.first().cloned().unwrap_or_default();
    let status_value = args.get(1).cloned().unwrap_or_default();
    let mut delivery = Delivery::default();
    if let Some(tests) = tests_value.as_object() {
        let length = tests
            .get(js_string!("length"), context)?
            .to_length(context)?;
        for i in 0..length.min(MAX_DELIVERED_TESTS) {
            let test = tests.get(i, context)?;
            let Some(test) = test.as_object() else {
                continue;
            };
            let name = string_property(&test, "name", context)?;
            let status = test
                .get(js_string!("status"), context)?
                .to_number(context)?;
            let message = string_property(&test, "message", context)?;
            delivery.tests.push(test_outcome(name, status, message));
        }
    }
    if let Some(status) = status_value.as_object() {
        delivery.harness_status = status
            .get(js_string!("status"), context)?
            .to_number(context)?;
        delivery.harness_message = string_property(&status, "message", context)?;
    }
    context.insert_data(delivery);
    Ok(JsValue::undefined())
}

/// `object[key]` as a string, with `null`/`undefined` read as empty.
fn string_property(
    object: &boa_engine::JsObject,
    key: &str,
    context: &mut Context,
) -> JsResult<String> {
    let value = object.get(JsString::from(key), context)?;
    if value.is_null_or_undefined() {
        return Ok(String::new());
    }
    Ok(value.to_string(context)?.to_std_string_escaped())
}

/// testharness.js's per-test status word (`Test.statuses`), or `None` for
/// PASS (0).
fn status_word(status: f64) -> Option<&'static str> {
    if status == 0.0 {
        return None;
    }
    Some(if status == 2.0 {
        "TIMEOUT"
    } else if status == 3.0 {
        "NOTRUN"
    } else if status == 4.0 {
        "PRECONDITION_FAILED"
    } else {
        "FAIL"
    })
}

/// One subtest's outcome: passed only for PASS; any other status puts its
/// word ahead of the harness's own message.
fn test_outcome(name: String, status: f64, message: String) -> TestOutcome {
    match status_word(status) {
        None => TestOutcome {
            name,
            passed: true,
            message,
        },
        Some(word) => TestOutcome {
            name,
            passed: false,
            message: format!("{word}: {message}"),
        },
    }
}

/// testharness.js's harness status word (`TestsStatus.statuses`), or `None`
/// for OK (0).
fn harness_status_word(status: f64) -> Option<&'static str> {
    if status == 0.0 {
        return None;
    }
    Some(if status == 2.0 {
        "TIMEOUT"
    } else if status == 3.0 {
        "PRECONDITION_FAILED"
    } else {
        "ERROR"
    })
}

/// Turn a finished run and whatever the harness delivered into the page's
/// result (precedence as documented on [`run_testharness_page`]).
fn page_outcome(
    report: &RunReport,
    delivery: Option<Delivery>,
) -> Result<Vec<TestOutcome>, PageError> {
    if let Some(reason) = &report.aborted {
        return Err(PageError::Aborted(reason.to_string()));
    }
    if !report.host_failures.is_empty() {
        return Err(PageError::Host(report.host_failures.join("; ")));
    }
    let Some(delivery) = delivery else {
        // The harness never completed. A script that failed to load (most
        // often testharness.js itself) explains that better than "no tests".
        if !report.fetch_errors.is_empty() {
            return Err(PageError::Host(report.fetch_errors.join("; ")));
        }
        return Err(PageError::NoResults);
    };
    if let Some(word) = harness_status_word(delivery.harness_status) {
        return Err(PageError::Harness(format!(
            "{word}: {}",
            delivery.harness_message
        )));
    }
    if delivery.tests.is_empty() {
        return Err(PageError::NoResults);
    }
    Ok(delivery.tests)
}

#[cfg(test)]
mod tests;
