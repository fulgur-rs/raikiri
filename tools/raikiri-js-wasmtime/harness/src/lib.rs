//! Boa-specific WPT support shared by native and isolated execution.
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{Context, JsResult, JsString, JsSymbol, JsValue, NativeFunction, js_string};
use raikiri_js::TestOutcome;
use raikiri_js::runtime::{DomRuntime, RunReport, RuntimeError};
pub const SINK_SYMBOL_DESCRIPTION: &str = "raikiri testharness report sink";

pub const REPORT_SCRIPT: &str = r#"(function (__raikiri_deliver) {
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

pub const DOCUMENT_FONTS_SCRIPT: &str = r#"(function () {
    var fonts = {
        load: function () { return Promise.resolve([]); }
    };
    fonts.ready = Promise.resolve(fonts);
    document.fonts = fonts;
})();
"#;

pub const MAX_DELIVERED_TESTS: u64 = 100_000;

#[derive(Debug, Default)]
pub struct Delivery {
    pub tests: Vec<TestOutcome>,
    pub harness_status: f64,
    pub harness_message: String,
}

pub fn take_delivery(runtime: &mut DomRuntime) -> Option<Delivery> {
    runtime
        .context_mut()
        .remove_data::<Delivery>()
        .map(|delivery| *delivery)
}

pub fn probe_timeout(runtime: &mut DomRuntime, report: &mut RunReport) -> Option<Delivery> {
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

pub fn install_page_support(
    runtime: &mut DomRuntime,
) -> Result<(), raikiri_js::runtime::RuntimeError> {
    runtime.evaluate(DOCUMENT_FONTS_SCRIPT)?;
    install_result_sink(runtime.context_mut())
        .map_err(|error| raikiri_js::runtime::RuntimeError::JavaScript(error.to_string())) // cov:ignore: defining a fresh symbol-keyed property on the global object does not fail.
}

pub fn install_result_sink(context: &mut Context) -> JsResult<()> {
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

pub fn deliver(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
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

pub fn string_property(
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

pub fn status_word(status: f64) -> Option<&'static str> {
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

pub fn test_outcome(name: String, status: f64, message: String) -> TestOutcome {
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

pub fn harness_status_word(status: f64) -> Option<&'static str> {
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
