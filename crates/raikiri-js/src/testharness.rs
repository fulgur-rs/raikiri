//! A small WPT testharness compatibility layer over the reusable DOM facade.
//!
//! The harness helpers here are intentionally separate from [`crate::dom`]:
//! future script runners can reuse the same JavaScript-to-DOM boundary without
//! using this test-specific assertion/reporting shim.

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsResult, js_string};

use crate::TestOutcome;
use crate::dom::{DomBackend, JsRuntime, ScriptError};

/// Why a testharness script could not produce a trustworthy result.
#[derive(Debug)]
pub enum TestHarnessError {
    /// Boa could not evaluate the harness, test source, or deferred font callback.
    JavaScript(String),
    /// A DOM operation or layout read failed in the host backend.
    Dom(String),
    /// The source did not register any `test()` calls.
    NoTests,
    /// The script kept scheduling callbacks beyond the bounded adapter loop.
    EventLoopLimit,
}

impl std::fmt::Display for TestHarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JavaScript(message) => write!(f, "JavaScript error: {message}"),
            Self::Dom(message) => write!(f, "DOM/layout error: {message}"),
            Self::NoTests => write!(f, "no test() calls in inline script"),
            Self::EventLoopLimit => write!(f, "testharness adapter exceeded its callback limit"),
        }
    }
}

impl std::error::Error for TestHarnessError {}

const TESTHARNESS_SHIM: &str = r#"
var __raikiri_font_callbacks = [];
var __raikiri_results = [];
if (typeof String.prototype.substr !== "function") {
    String.prototype.substr = function (start, length) {
        var value = String(this);
        var from = Number(start);
        if (!Number.isFinite(from)) from = 0;
        from = from < 0 ? Math.max(value.length + Math.trunc(from), 0) : Math.min(Math.trunc(from), value.length);
        if (length === undefined) return value.slice(from);
        var count = Number(length);
        if (!Number.isFinite(count) || count <= 0) return "";
        return value.slice(from, from + Math.trunc(count));
    };
}
document.fonts = {
    load: function (_font, _text) {
        return {
            then: function (callback) {
                if (typeof callback !== "function") {
                    throw new TypeError("document.fonts.load(...).then expects a function");
                }
                __raikiri_font_callbacks.push(function () {
                    callback([]);
                });
                return this;
            }
        };
    },
    ready: {
        then: function (callback) {
            if (typeof callback !== "function") {
                throw new TypeError("document.fonts.ready.then expects a function");
            }
            __raikiri_font_callbacks.push(callback);
            return this;
        }
    }
};
function setup(_options) {}
function done() {}
function test(fn, name) {
    try {
        fn();
        __raikiri_results.push({ name: String(name), passed: true, message: "" });
    } catch (error) {
        __raikiri_results.push({ name: String(name), passed: false, message: String(error) });
    }
}
function assert_true(actual, message) {
    if (!actual) {
        throw new Error((message || "assert_true") + ": expected a truthy value");
    }
}
function assert_equals(actual, expected, message) {
    if (actual !== expected) {
        throw new Error((message || "assert_equals") + ": expected " +
            String(expected) + ", got " + String(actual));
    }
}
function assert_in_array(actual, expectedArray, message) {
    if (!Array.isArray(expectedArray) || expectedArray.indexOf(actual) === -1) {
        throw new Error((message || "assert_in_array") + ": value " +
            JSON.stringify(actual) + " not in array " + JSON.stringify(expectedArray));
    }
}
function assert_approx_equals(actual, expected, epsilon, message) {
    if (typeof actual !== "number" || typeof expected !== "number" ||
        typeof epsilon !== "number" || !Number.isFinite(actual) ||
        !Number.isFinite(expected) || !Number.isFinite(epsilon) ||
        Math.abs(actual - expected) > epsilon) {
        throw new Error((message || "assert_approx_equals") +
            ": expected " + expected + " ± " + epsilon + ", got " + actual);
    }
}
function __raikiri_run_font_callbacks(limit) {
    var invoked = 0;
    while (__raikiri_font_callbacks.length > 0 && invoked < limit) {
        invoked += 1;
        __raikiri_font_callbacks.shift()();
    }
}
"#;

const FONT_CALLBACKS_PER_TURN: usize = 64;
const MAX_FONT_CALLBACK_TURNS: usize = 16;

/// Run an unmodified inline WPT script using the supplied DOM implementation
/// and a small testharness API shim.
///
/// DOM queries, mutations, and geometry reads go through [`DomBackend`]. The
/// caller can change its backend without changing the script source or this
/// runner's JavaScript-facing DOM facade.
pub fn run_testharness_script<B>(
    inline_script: &str,
    backend: B,
) -> Result<Vec<TestOutcome>, TestHarnessError>
where
    B: DomBackend,
{
    let mut runtime = JsRuntime::new(backend).map_err(map_script_error)?;
    runtime
        .evaluate(TESTHARNESS_SHIM)
        .map_err(map_script_error)?;
    runtime.evaluate(inline_script).map_err(map_script_error)?;

    for _ in 0..MAX_FONT_CALLBACK_TURNS {
        if has_pending_font_callbacks(runtime.context_mut())
            .map_err(|error| TestHarnessError::JavaScript(error.to_string()))?
        {
            runtime
                .evaluate(&format!(
                    "__raikiri_run_font_callbacks({FONT_CALLBACKS_PER_TURN})"
                ))
                .map_err(map_script_error)?;
        }

        let callbacks_pending = has_pending_font_callbacks(runtime.context_mut())
            .map_err(|error| TestHarnessError::JavaScript(error.to_string()))?;
        if !callbacks_pending {
            let results = read_results(runtime.context_mut())
                .map_err(|error| TestHarnessError::JavaScript(error.to_string()))?;
            return if results.is_empty() {
                Err(TestHarnessError::NoTests)
            } else {
                Ok(results)
            };
        }
    }

    Err(TestHarnessError::EventLoopLimit)
}

fn map_script_error(error: ScriptError) -> TestHarnessError {
    match error {
        ScriptError::JavaScript(message) => TestHarnessError::JavaScript(message),
        ScriptError::Dom(message) => TestHarnessError::Dom(message),
    }
}

fn has_pending_font_callbacks(context: &mut Context) -> JsResult<bool> {
    let value = context
        .global_object()
        .get(js_string!("__raikiri_font_callbacks"), context)?;
    let object = value.as_object().expect("font callback queue is an array");
    let array = JsArray::from_object(object.clone())?;
    Ok(array.length(context)? > 0)
}

fn read_results(context: &mut Context) -> JsResult<Vec<TestOutcome>> {
    let value = context
        .global_object()
        .get(js_string!("__raikiri_results"), context)?;
    let object = value.as_object().expect("test results are an array");
    let array = JsArray::from_object(object.clone())?;
    let len = array.length(context)?;
    let mut outcomes = Vec::with_capacity(len as usize);
    for index in 0..len {
        let entry = array.get(index, context)?;
        let object = entry.as_object().expect("test result is an object");
        outcomes.push(TestOutcome {
            name: object
                .get(js_string!("name"), context)?
                .to_string(context)?
                .to_std_string_escaped(),
            passed: object.get(js_string!("passed"), context)?.to_boolean(),
            message: object
                .get(js_string!("message"), context)?
                .to_string(context)?
                .to_std_string_escaped(),
        });
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests;
