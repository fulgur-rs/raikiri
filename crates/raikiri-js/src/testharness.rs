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
function assert_approx_equals(actual, expected, epsilon, message) {
    if (typeof actual !== "number" || typeof expected !== "number" ||
        typeof epsilon !== "number" || !Number.isFinite(actual) ||
        !Number.isFinite(expected) || !Number.isFinite(epsilon) ||
        Math.abs(actual - expected) > epsilon) {
        throw new Error((message || "assert_approx_equals") +
            ": expected " + expected + " ± " + epsilon + ", got " + actual);
    }
}
function __raikiri_run_font_callbacks() {
    while (__raikiri_font_callbacks.length > 0) {
        __raikiri_font_callbacks.shift()();
    }
}
"#;

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

    for _ in 0..16 {
        if has_pending_font_callbacks(runtime.context_mut())
            .map_err(|error| TestHarnessError::JavaScript(error.to_string()))?
        {
            runtime
                .evaluate("__raikiri_run_font_callbacks()")
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
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::dom::{DomNodeId, DomRect, ElementGeometry};

    #[derive(Default)]
    struct TestDom {
        by_id: BTreeMap<String, (DomNodeId, ElementGeometry)>,
        body_html: String,
        styles: BTreeMap<(DomNodeId, String), String>,
        fail_geometry: bool,
    }

    impl TestDom {
        fn with_element(mut self, id: &str, node: DomNodeId, geometry: ElementGeometry) -> Self {
            self.by_id.insert(id.to_owned(), (node, geometry));
            self
        }

        fn with_failing_geometry(mut self) -> Self {
            self.fail_geometry = true;
            self
        }

        fn geometry(&self, node: DomNodeId) -> Option<ElementGeometry> {
            self.by_id
                .values()
                .find_map(|(candidate, geometry)| (*candidate == node).then_some(*geometry))
        }
    }

    impl DomBackend for TestDom {
        fn get_element_by_id(&mut self, id: &str) -> Result<Option<DomNodeId>, String> {
            Ok(self.by_id.get(id).map(|(node, _)| *node))
        }

        fn query_selector(&mut self, selector: &str) -> Result<Option<DomNodeId>, String> {
            Ok((selector.eq_ignore_ascii_case("body")).then_some(0))
        }

        fn parent_node(&mut self, node: DomNodeId) -> Result<Option<DomNodeId>, String> {
            Ok((node != 0).then_some(0))
        }

        fn offset_height(&mut self, node: DomNodeId) -> Result<f64, String> {
            if self.fail_geometry {
                return Err("test geometry backend failure".into());
            }
            self.geometry(node)
                .map(|geometry| geometry.offset_height)
                .ok_or_else(|| format!("no layout for node {node}"))
        }

        fn bounding_client_rect(&mut self, node: DomNodeId) -> Result<DomRect, String> {
            if self.fail_geometry {
                return Err("test geometry backend failure".into());
            }
            let geometry = self
                .geometry(node)
                .ok_or_else(|| format!("no layout for node {node}"))?;
            Ok(DomRect {
                left: geometry.left,
                right: geometry.left,
                height: geometry.offset_height,
                ..DomRect::default()
            })
        }

        fn inner_html(&mut self, _node: DomNodeId) -> Result<String, String> {
            Ok(self.body_html.clone())
        }

        fn set_inner_html(&mut self, _node: DomNodeId, value: &str) -> Result<(), String> {
            self.body_html = value.to_owned();
            Ok(())
        }

        fn style_property(&mut self, node: DomNodeId, property: &str) -> Result<String, String> {
            Ok(self
                .styles
                .get(&(node, property.to_owned()))
                .cloned()
                .unwrap_or_default())
        }

        fn set_style_property(
            &mut self,
            node: DomNodeId,
            property: &str,
            value: &str,
        ) -> Result<(), String> {
            self.styles
                .insert((node, property.to_owned()), value.to_owned());
            Ok(())
        }
    }

    #[test]
    fn general_script_runtime_preserves_global_scope_across_evaluations() {
        let mut runtime = JsRuntime::new(TestDom::default()).unwrap();
        runtime.evaluate("var shared = 9;").unwrap();
        runtime
            .evaluate("if (shared !== 9) throw new Error('global scope was reset');")
            .unwrap();
    }

    #[test]
    fn raw_script_uses_the_dom_facade_without_testharness_globals() {
        let backend = TestDom::default().with_element(
            "line",
            1,
            ElementGeometry {
                offset_height: 60.0,
                left: 12.0,
            },
        );
        crate::dom::run_script(
            "if (document.body === null || document.getElementById('line').offsetHeight !== 60) throw new Error('DOM binding failed');",
            backend,
        )
        .unwrap();
    }

    #[test]
    fn runs_assertions_against_the_dom_backend() {
        let backend = TestDom::default().with_element(
            "line",
            1,
            ElementGeometry {
                offset_height: 60.0,
                left: 12.0,
            },
        );
        let outcomes = run_testharness_script(
            "test(function() { assert_true(document.getElementById('line').offsetHeight > 35); }, 'height');",
            backend,
        )
        .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].passed, "{:?}", outcomes[0]);
    }

    #[test]
    fn missing_element_is_an_assertion_failure_not_a_pass() {
        let result = run_testharness_script(
            "test(function() { assert_true(document.getElementById('missing').offsetHeight > 35); }, 'missing');",
            TestDom::default(),
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert!(!result[0].passed);
        assert!(result[0].message.contains("null"));
    }

    #[test]
    fn inner_html_mutation_is_visible_to_deferred_font_callback() {
        let backend = TestDom::default().with_element(
            "span",
            2,
            ElementGeometry {
                offset_height: 30.0,
                left: 42.0,
            },
        );
        let result = run_testharness_script(
            r#"
                document.querySelector('body').innerHTML = '<span id="span">text</span>';
                setup({explicit_done: true});
                document.fonts.ready.then(function() {
                    test(function() {
                        assert_approx_equals(document.getElementById('span').getBoundingClientRect().left, 42, 1);
                        document.getElementById('span').parentNode.style.display = 'none';
                    }, 'left');
                    done();
                });
            "#,
            backend,
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].passed, "{:?}", result[0]);
    }

    #[test]
    fn backend_failures_are_reported_as_dom_errors() {
        let backend = TestDom::default()
            .with_element(
                "broken",
                3,
                ElementGeometry {
                    offset_height: 0.0,
                    left: 0.0,
                },
            )
            .with_failing_geometry();
        // The test harness catches the JS exception. The adapter still reports
        // the underlying failed geometry lookup as a DOM error, not an assertion.
        let result = run_testharness_script(
            "test(function() { document.getElementById('broken').offsetHeight; }, 'broken geometry');",
            backend,
        );
        assert!(matches!(result, Err(TestHarnessError::Dom(_))));
    }
}
