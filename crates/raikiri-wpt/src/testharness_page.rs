//! Run one WPT testharness page end to end with the real, unmodified
//! `resources/testharness.js` from the checkout: the page's own `<script>`
//! elements run in document order through [`raikiri_js::runtime::DomRuntime::run_document`], and
//! this crate's report script (served in place of
//! `resources/testharnessreport.js`, see [`REPORT_SCRIPT`]) hands the
//! harness's completion results back to Rust.

use raikiri_js_wasmtime_harness::{Delivery, harness_status_word};
#[cfg(feature = "js-native")]
use raikiri_js_wasmtime_harness::{install_page_support, probe_timeout, take_delivery};
use std::path::Path;
#[cfg(feature = "js-wasmtime")]
mod backend;
#[cfg(all(feature = "js-native", feature = "js-wasmtime"))]
compile_error!("select only one JS backend");
#[cfg(not(any(feature = "js-native", feature = "js-wasmtime")))]
compile_error!("select a JS backend");
pub(crate) use raikiri_js_wasmtime_harness::REPORT_SCRIPT;
#[cfg(test)]
use raikiri_js_wasmtime_harness::SINK_SYMBOL_DESCRIPTION;

use raikiri_js::TestOutcome;
#[cfg(feature = "js-native")]
use raikiri_js::runtime::DomRuntime;
use raikiri_js::runtime::RunReport;

use crate::reftest::{DEFAULT_REFTTEST_HEIGHT, DEFAULT_REFTTEST_WIDTH, prepare_wpt_live_document};
use crate::wpt_host::WptDocumentHost;

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
    /// for a forced `timeout()` (see [`raikiri_js_wasmtime_harness::probe_timeout`]) to report either.
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

/// Run one WPT testharness page with the real testharness.js through
/// `run_document`. `path` is the page's file, relative to `wpt_root` (an
/// absolute path works too).
///
/// When several things went wrong at once the most fundamental one is
/// reported, since it makes everything after it untrustworthy: a resource
/// limit abort, then a host failure, then (when the harness never
/// completed) a script that failed to load, then a non-OK harness status,
/// then an empty result list.
#[cfg(feature = "js-native")]
pub(crate) fn run_testharness_page(
    path: &Path,
    wpt_root: &Path,
) -> Result<Vec<TestOutcome>, PageError> {
    let mut runtime = prepare_page(path, wpt_root)?;
    let initial = metrics_initial_rss();
    let result = finish_page(&mut runtime);
    trace_native_rss(path, initial);
    result
}

/// Like [`run_testharness_page`], but first evaluates `preamble` as a
/// classic script of its own, after the page's document is built and before
/// any of the page's `<script>` elements run. An uncaught error in it stops
/// the page there, reported as [`PageError::Preamble`], so a failed sanity
/// check never mixes with the page's own results.
#[cfg(feature = "js-native")]
pub(crate) fn run_testharness_page_with_preamble(
    path: &Path,
    wpt_root: &Path,
    preamble: &str,
) -> Result<Vec<TestOutcome>, PageError> {
    let mut runtime = prepare_page(path, wpt_root)?;
    let initial = metrics_initial_rss();
    let result = (|| {
        runtime
            .evaluate(preamble)
            .map_err(|error| PageError::Preamble(error.to_string()))?;
        finish_page(&mut runtime)
    })();
    trace_native_rss(path, initial);
    result
}

/// Read the page, build its live document and a runtime over it, and install
/// the page support scripts; no page script has run yet.
#[cfg(feature = "js-native")]
fn prepare_page(path: &Path, wpt_root: &Path) -> Result<DomRuntime, PageError> {
    let host = prepare_host(path, wpt_root)?;
    let mut runtime = DomRuntime::new(host).map_err(|error| PageError::Host(error.to_string()))?; // cov:ignore: building a realm over a parsed document does not fail.
    let installed = install_page_support(&mut runtime);
    installed.map_err(|error| PageError::Host(error.to_string()))?; // cov:ignore: only defines properties on a fresh realm's own objects.
    Ok(runtime)
}

fn prepare_host(path: &Path, wpt_root: &Path) -> Result<WptDocumentHost, PageError> {
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
    Ok(host)
}

/// Run the page's scripts and turn the run and whatever the harness
/// delivered into the page's result.
///
/// A page can run to completion (no abort, no host failure, no failed script
/// fetch) without the harness ever delivering anything:
/// `setup({explicit_timeout: true, ...})` (see [`REPORT_SCRIPT`]) disables
/// testharness.js's own self-timeout, so an `async_test`/`promise_test` that
/// never finishes waiting leaves nothing to read back, indistinguishable on
/// its own from a page with zero tests. In that case only, [`raikiri_js_wasmtime_harness::probe_timeout`]
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
/// leave alone (see [`raikiri_js_wasmtime_harness::probe_timeout`]'s own doc comment), so it falls through
/// to [`PageError::NoResults`] as if the probe had never run.
#[cfg(feature = "js-native")]
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

#[cfg(feature = "js-wasmtime")]
pub(crate) fn run_testharness_page(
    path: &Path,
    wpt_root: &Path,
) -> Result<Vec<TestOutcome>, PageError> {
    backend::run(path, wpt_root, None)
}
#[cfg(feature = "js-wasmtime")]
pub(crate) fn run_testharness_page_with_preamble(
    path: &Path,
    wpt_root: &Path,
    preamble: &str,
) -> Result<Vec<TestOutcome>, PageError> {
    backend::run(path, wpt_root, Some(preamble))
}

fn process_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("VmRSS:")
                    .and_then(|s| s.split_whitespace().next()?.parse().ok())
            })
        })
        .unwrap_or(0)
}
#[cfg(feature = "js-native")]
fn metrics_initial_rss() -> Option<u64> {
    std::env::var_os("RAIKIRI_WPT_METRICS").map(|_| process_rss_kib())
}
#[cfg(feature = "js-native")]
fn trace_native_rss(path: &Path, initial: Option<u64>) {
    if let Some(initial) = initial {
        eprintln!(
            "NATIVE_METRICS {} rss_initial_kib={} rss_steady_kib={}",
            path.display(),
            initial,
            process_rss_kib()
        );
    }
}
