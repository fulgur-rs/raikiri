//! Run a WPT CSS parsing test script inside a pure-Rust JS engine (Boa),
//! backed by `raikiri_style::property::parse_value`/`serialize_value`/
//! `serialize_color_value` for the actual CSS validity and serialization
//! decisions.
//!
//! Scope: `test_invalid_value` and the parts of `test_valid_value` that
//! `raikiri_style::property::serialize_value`/`serialize_color_value` cover
//! (falls back to echoing the input for the rest — see those functions'
//! doc comments). `test_valid_selector`/`test_valid_rule` need a
//! `CSSStyleSheet`/`CSSRule` surface this crate doesn't implement.

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsResult, JsValue, NativeFunction, Source, js_string};
use cssparser::{ParseError, Parser, ParserInput};
use raikiri_style::property::PropertyValue;

/// One WPT `test()` call's outcome, as recorded by the JS-side harness shim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    /// The `test()` call's name argument, verbatim.
    pub name: String,
    /// Whether the test function ran without throwing.
    pub passed: bool,
    /// The thrown error's string form, empty when `passed` is true.
    pub message: String,
}

/// Why a script run could not produce a trustworthy result.
#[derive(Debug)]
pub enum HarnessError {
    /// The positive control (see [`run_invalid_value_script`]) failed
    /// before any real assertion ran, so a clean run would say nothing
    /// about the CSS binding's correctness.
    PositiveControlFailed(String),
    /// The JS engine reported an uncaught error while evaluating shim or
    /// test source.
    Js(String),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessError::PositiveControlFailed(msg) => {
                write!(f, "positive control failed: {msg}")
            }
            HarnessError::Js(msg) => write!(f, "JS error: {msg}"),
        }
    }
}

impl std::error::Error for HarnessError {}

/// The only native hook the harness needs: does `raikiri_style` accept
/// `args[1]` as a value for the CSS property named `args[0]`, and if so,
/// what should `getPropertyValue` read back? Returns `null` when the value
/// is invalid; otherwise the real canonical serialization when
/// `raikiri_style::property::serialize_value` or `serialize_color_value`
/// covers this variant, or the raw input echoed back when neither does
/// (yet) — see those functions' doc comments for what "doesn't (yet)"
/// covers. Called from the JS-side `Proxy` `set` trap defined in
/// [`SHIM_JS`].
fn parse_and_serialize_property_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = args
        .first()
        .cloned()
        .unwrap_or_default()
        .to_string(context)?
        .to_std_string_escaped();
    let raw_value = args
        .get(1)
        .cloned()
        .unwrap_or_default()
        .to_string(context)?
        .to_std_string_escaped();
    let value: Option<PropertyValue> = {
        let mut input = ParserInput::new(&raw_value);
        let mut parser = Parser::new(&mut input);
        parser
            .parse_entirely(|i| -> Result<PropertyValue, ParseError<'_, ()>> {
                raikiri_style::property::parse_value(&name, i).ok_or_else(|| i.new_custom_error(()))
            })
            .ok()
    };
    let Some(value) = value else {
        return Ok(JsValue::null());
    };
    let serialized = raikiri_style::property::serialize_value(&value)
        .or_else(|| raikiri_style::property::serialize_color_value(&name, &raw_value))
        .unwrap_or(raw_value);
    Ok(JsValue::from(js_string!(serialized)))
}

/// Harness shim: a `document`/`style` stand-in plus the small subset of
/// `testharness.js` that `parsing-testcommon.js`'s `test_invalid_value`/
/// `test_valid_value` actually call (`test`, `assert_equals`,
/// `assert_not_equals`). `style` is a standard ECMAScript `Proxy` whose
/// `get`/`set` traps forward to [`parse_and_serialize_property_native`] by
/// property name, so no property needs its own JS declaration — any
/// property `raikiri_style::property::parse_value` recognizes works
/// without a code change here.
const SHIM_JS: &str = r#"
function makeStyle() {
    var backing = {};
    return new Proxy({}, {
        get: function (target, prop) {
            if (prop === "getPropertyValue") {
                return function (name) {
                    return backing[name] === undefined ? "" : backing[name];
                };
            }
            return backing[prop] === undefined ? "" : backing[prop];
        },
        set: function (target, prop, value) {
            var result = value === "" ? null : __raikiri_parse_and_serialize_property(prop, value);
            if (result === null) {
                delete backing[prop];
            } else {
                backing[prop] = result;
            }
            return true;
        }
    });
}

var document = {
    getElementById: function () { return null; },
    createElement: function () { return { style: makeStyle() }; }
};

var __results = [];

function test(fn, name) {
    try {
        fn();
        __results.push({ name: name, passed: true, message: "" });
    } catch (e) {
        __results.push({ name: name, passed: false, message: String(e) });
    }
}

function assert_equals(actual, expected, message) {
    if (actual !== expected) {
        throw new Error(
            (message || "assert_equals") + ": expected " + JSON.stringify(expected) +
            " got " + JSON.stringify(actual)
        );
    }
}

function assert_not_equals(actual, notExpected, message) {
    if (actual === notExpected) {
        throw new Error((message || "assert_not_equals") + ": did not expect " + JSON.stringify(notExpected));
    }
}
"#;

/// Known-valid, property-agnostic sanity check: `color: red` is pinned as
/// valid by `raikiri-style`'s own test suite, independent of whatever
/// property the real test script under evaluation exercises. Its only job
/// is to catch a completely inert binding (the `Proxy` `set` trap never
/// firing, [`parse_and_serialize_property_native`] always returning
/// `null`, and so on) before any real assertion is trusted — see
/// [`run_invalid_value_script`].
const POSITIVE_CONTROL_JS: &str = r#"
(function () {
    var div = document.createElement("div");
    div.style["color"] = "red";
    var got = div.style.getPropertyValue("color");
    if (got !== "red") {
        throw new Error("color round-trip failed, got " + JSON.stringify(got));
    }
})();
"#;

fn new_context() -> JsResult<Context> {
    let mut context = Context::default();
    context.register_global_callable(
        js_string!("__raikiri_parse_and_serialize_property"),
        2,
        NativeFunction::from_fn_ptr(parse_and_serialize_property_native),
    )?;
    context.eval(Source::from_bytes(SHIM_JS))?;
    Ok(context)
}

/// Run `parsing_testcommon_js` (the real, unmodified WPT helper source)
/// followed by `test_script` (a WPT parsing test file's inline `<script>`
/// body) inside a fresh engine instance.
///
/// Before trusting any result, this runs a positive control: setting
/// `style["color"]` to `"red"` must read back `"red"` through the same
/// JS-visible path the real assertions use. If the CSSOM binding's setter
/// silently no-ops (or the native hook always accepts/rejects regardless of
/// input), every assertion in `test_script` could pass or fail for the
/// wrong reason; a positive-control failure is reported instead of a
/// result that isn't backed by a working binding.
pub fn run_invalid_value_script(
    parsing_testcommon_js: &str,
    test_script: &str,
) -> Result<Vec<TestOutcome>, HarnessError> {
    let mut context = new_context().map_err(|e| HarnessError::Js(e.to_string()))?;

    context
        .eval(Source::from_bytes(POSITIVE_CONTROL_JS))
        .map_err(|e| HarnessError::PositiveControlFailed(e.to_string()))?;

    context
        .eval(Source::from_bytes(parsing_testcommon_js))
        .map_err(|e| HarnessError::Js(e.to_string()))?;
    context
        .eval(Source::from_bytes(test_script))
        .map_err(|e| HarnessError::Js(e.to_string()))?;

    read_results(&mut context).map_err(|e| HarnessError::Js(e.to_string()))
}

fn read_results(context: &mut Context) -> JsResult<Vec<TestOutcome>> {
    let results_value = context
        .global_object()
        .get(js_string!("__results"), context)?;
    let results_obj = results_value
        .as_object()
        .expect("__results is defined as an array by SHIM_JS");
    let array = JsArray::from_object(results_obj)?;
    let len = array.length(context)?;

    let mut outcomes = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = array.get(i, context)?;
        let item_obj = item.as_object().expect("__results entries are objects");
        let name = item_obj
            .get(js_string!("name"), context)?
            .to_string(context)?
            .to_std_string_escaped();
        let passed = item_obj.get(js_string!("passed"), context)?.to_boolean();
        let message = item_obj
            .get(js_string!("message"), context)?
            .to_string(context)?
            .to_std_string_escaped();
        outcomes.push(TestOutcome {
            name,
            passed,
            message,
        });
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_control_alone_is_green() {
        let mut context = new_context().unwrap();
        assert!(
            context
                .eval(Source::from_bytes(POSITIVE_CONTROL_JS))
                .is_ok()
        );
    }

    /// If [`parse_and_serialize_property_native`] were stubbed to always
    /// reject (or the `Proxy` `set` trap never called it at all), the
    /// positive control itself — not just the real assertions — must fail.
    /// This is what makes a run's eventual PASS/FAIL trustworthy rather
    /// than vacuous.
    #[test]
    fn positive_control_catches_a_stub_binding() {
        let mut context = Context::default();
        context
            .register_global_callable(
                js_string!("__raikiri_parse_and_serialize_property"),
                2,
                NativeFunction::from_fn_ptr(
                    |_this: &JsValue, _args: &[JsValue], _ctx: &mut Context| Ok(JsValue::null()),
                ),
            )
            .unwrap();
        context.eval(Source::from_bytes(SHIM_JS)).unwrap();
        let result = context.eval(Source::from_bytes(POSITIVE_CONTROL_JS));
        assert!(
            result.is_err(),
            "a stub binding that rejects everything must fail the positive control"
        );
    }

    #[test]
    fn rejects_known_invalid_values_across_properties() {
        let mut context = new_context().unwrap();
        for (name, bad) in [
            ("box-sizing", "margin-box"),
            ("box-sizing", "bogus"),
            ("display", "not-a-real-display-keyword"),
        ] {
            let script = format!("__raikiri_parse_and_serialize_property({name:?}, {bad:?})");
            let result = context.eval(Source::from_bytes(script.as_bytes())).unwrap();
            assert!(
                result.is_null(),
                "expected ({name:?}, {bad:?}) to be rejected"
            );
        }
    }

    #[test]
    fn accepts_known_valid_values_across_properties() {
        let mut context = new_context().unwrap();
        for (name, good) in [
            ("box-sizing", "content-box"),
            ("display", "block"),
            ("color", "red"),
        ] {
            let script = format!("__raikiri_parse_and_serialize_property({name:?}, {good:?})");
            let result = context.eval(Source::from_bytes(script.as_bytes())).unwrap();
            assert!(
                !result.is_null(),
                "expected ({name:?}, {good:?}) to be accepted"
            );
        }
    }

    /// Companion to the positive control: feeds a *valid* value through the
    /// exact assert shape `test_invalid_value` uses (set, then assert the
    /// read-back is empty). This must come out FAIL. If `assert_equals` or
    /// `test`'s catch branch were silently inert, this would report `passed:
    /// true` instead, which is exactly the failure mode that would make a
    /// real fixture's all-PASS result meaningless.
    #[test]
    fn must_fail_control_a_valid_value_is_reported_as_failed_by_invalid_check() {
        let outcomes = run_invalid_value_script(
            "",
            r#"test(function () {
                var div = document.createElement("div");
                div.style["box-sizing"] = "";
                div.style["box-sizing"] = "content-box";
                assert_equals(div.style.getPropertyValue("box-sizing"), "");
            }, "must-fail control");"#,
        )
        .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(
            !outcomes[0].passed,
            "harness is broken: a valid box-sizing value was reported as matching \
             the invalid-value (empty read-back) expectation"
        );
    }

    #[test]
    fn generic_proxy_harness_handles_two_different_properties() {
        let box_sizing_outcomes = run_invalid_value_script(
            "",
            r#"test(function () {
                var div = document.createElement("div");
                div.style["box-sizing"] = "";
                div.style["box-sizing"] = "margin-box";
                assert_equals(div.style.getPropertyValue("box-sizing"), "");
            }, "box-sizing margin-box rejected");"#,
        )
        .unwrap();
        assert_eq!(box_sizing_outcomes.len(), 1);
        assert!(
            box_sizing_outcomes[0].passed,
            "{:?}",
            box_sizing_outcomes[0]
        );

        let display_outcomes = run_invalid_value_script(
            "",
            r#"test(function () {
                var div = document.createElement("div");
                div.style["display"] = "";
                div.style["display"] = "not-a-real-display-keyword";
                assert_equals(div.style.getPropertyValue("display"), "");
            }, "display bogus keyword rejected");"#,
        )
        .unwrap();
        assert_eq!(display_outcomes.len(), 1);
        assert!(display_outcomes[0].passed, "{:?}", display_outcomes[0]);
    }

    #[test]
    fn valid_value_reads_back_the_real_serialization_not_the_raw_input() {
        let outcomes = run_invalid_value_script(
            "",
            r#"test(function () {
                var div = document.createElement("div");
                div.style["padding-top"] = "";
                div.style["padding-top"] = "010.0px";
                assert_equals(div.style.getPropertyValue("padding-top"), "10px");
            }, "padding-top serializes 010.0px as 10px");"#,
        )
        .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].passed, "{:?}", outcomes[0]);
    }

    #[test]
    fn valid_value_serializes_legacy_color_syntax_but_echoes_modern_syntax() {
        let outcomes = run_invalid_value_script(
            "",
            r##"test(function () {
                var div = document.createElement("div");
                div.style["color"] = "";
                div.style["color"] = "#234";
                assert_equals(div.style.getPropertyValue("color"), "rgb(34, 51, 68)");
            }, "color serializes hex as rgb()");
            test(function () {
                var div = document.createElement("div");
                div.style["color"] = "";
                div.style["color"] = "red";
                assert_equals(div.style.getPropertyValue("color"), "red");
            }, "color echoes a keyword back unchanged");
            test(function () {
                var div = document.createElement("div");
                div.style["color"] = "";
                div.style["color"] = "lab(0 0 0)";
                assert_equals(div.style.getPropertyValue("color"), "lab(0 0 0)");
            }, "color echoes modern syntax it cannot serialize");"##,
        )
        .unwrap();
        assert_eq!(outcomes.len(), 3);
        for outcome in &outcomes {
            assert!(outcome.passed, "{outcome:?}");
        }
    }
}
