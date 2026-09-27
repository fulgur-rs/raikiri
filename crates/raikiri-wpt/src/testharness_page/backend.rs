//! Compile-time Wasmtime page adapter with native outcome precedence.
use super::*;
use raikiri_js_wasmtime_host::{EngineError, SandboxOptions, WasmtimePage};
fn engine_error(e: EngineError) -> PageError {
    if e.aborted {
        PageError::Aborted(e.message)
    } else {
        PageError::Host(e.message)
    }
}
pub(super) fn run(
    path: &Path,
    wpt_root: &Path,
    preamble: Option<&str>,
) -> Result<Vec<TestOutcome>, PageError> {
    let host = prepare_host(path, wpt_root)?;
    let mut sandbox = SandboxOptions::default();
    if let Ok(fuel) = std::env::var("RAIKIRI_WPT_FUEL") {
        sandbox.fuel = fuel
            .parse()
            .map_err(|_| PageError::Host("invalid RAIKIRI_WPT_FUEL".into()))?;
    }
    if let Ok(memory) = std::env::var("RAIKIRI_WPT_MEMORY_BYTES") {
        sandbox.memory_bytes = memory
            .parse()
            .map_err(|_| PageError::Host("invalid RAIKIRI_WPT_MEMORY_BYTES".into()))?;
    }
    let measure = std::env::var_os("RAIKIRI_WPT_METRICS").is_some();
    let rss_before = measure.then(super::process_rss_kib);
    let mut runtime =
        WasmtimePage::new(Box::new(host), Default::default(), sandbox).map_err(engine_error)?;
    let rss_initial = measure.then(super::process_rss_kib);
    let result = (|| {
        if let Some(preamble) = preamble {
            runtime
                .evaluate_preamble(preamble)
                .map_err(|e| PageError::Preamble(e.to_string()))?;
        }
        let mut report = runtime.run_document().map_err(engine_error)?;
        let mut delivery = runtime.take_results().map_err(engine_error)?;
        if delivery.is_none()
            && report.aborted.is_none()
            && report.host_failures.is_empty()
            && report.fetch_errors.is_empty()
        {
            report = runtime.probe_timeout(report).map_err(engine_error)?;
            delivery = runtime
                .take_results()
                .map_err(engine_error)?
                .filter(|d| !d.tests.is_empty());
        }
        runtime.synchronize_final_document().map_err(engine_error)?;
        let delivery = delivery.map(|d| Delivery {
            tests: d
                .tests
                .into_iter()
                .map(|t| TestOutcome {
                    name: t.name,
                    passed: t.passed,
                    message: t.message,
                })
                .collect(),
            harness_status: d.harness_status,
            harness_message: d.harness_message,
        });
        page_outcome(&report.into_native(), delivery)
    })();
    if measure && let Some(m) = runtime.metrics() {
        eprintln!(
            "WASM_METRICS {} calls={} request_bytes={} response_bytes={} fuel={} memory_bytes={} guest_operations={} guest_request_bytes={} guest_response_bytes={} rss_before_kib={} rss_initial_kib={} rss_steady_kib={}",
            path.display(),
            m.bridge.calls,
            m.bridge.request_bytes,
            m.bridge.response_bytes,
            m.consumed_fuel,
            m.memory_bytes,
            m.guest_operations,
            m.guest_request_bytes,
            m.guest_response_bytes,
            rss_before.unwrap_or(0),
            rss_initial.unwrap_or(0),
            super::process_rss_kib()
        );
    }
    result
}
