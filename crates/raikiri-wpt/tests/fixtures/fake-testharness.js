// Shared fake `resources/testharness.js` for unit and bin tests.
//
// Covers the superset of what the hand-rolled stand-ins used before:
// `setup`, `add_completion_callback`, `test`, `assert_true`, `assert_equals`,
// `assert_approx_equals`, `done` with `setup({explicit_done: true})`,
// `report(tests, status)`, and an uncaught-error-to-harness-status mapping.
// Results are delivered to completion callbacks on `load`, or on `done()`
// when explicit completion was requested.
var __tests = [];
var __callbacks = [];
var __status = { status: 0, message: null };
var __explicit_done = false;
var __delivered = false;
function setup(options) {
    window.__setup_options = options;
    if (options && options.explicit_done) {
        __explicit_done = true;
    }
}
function add_completion_callback(callback) { __callbacks.push(callback); }
function assert_true(value, message) {
    if (value !== true) {
        throw new Error("assert_true: " + (message || String(value)));
    }
}
function assert_equals(actual, expected, message) {
    if (actual !== expected) {
        throw new Error("assert_equals: expected " + String(expected) + ", got " + String(actual) + (message ? ": " + message : ""));
    }
}
function assert_approx_equals(actual, expected, epsilon, message) {
    if (typeof actual !== "number" || typeof expected !== "number" || typeof epsilon !== "number") {
        throw new Error("assert_approx_equals: values must be numbers" + (message ? ": " + message : ""));
    }
    if (!(Math.abs(actual - expected) <= epsilon)) {
        throw new Error("assert_approx_equals: expected " + String(expected) + " +/- " + String(epsilon) + ", got " + String(actual) + (message ? ": " + message : ""));
    }
}
function test(body, name) {
    try {
        body();
        __tests.push({ name: name, status: 0, message: null });
    } catch (error) {
        __tests.push({ name: name, status: 1, message: String(error) });
    }
}
function done() {
    __deliver(__tests, __status);
}
function report(tests, status) {
    window.addEventListener("load", function () {
        for (var i = 0; i < __callbacks.length; i++) {
            __callbacks[i](tests, status);
        }
    });
}
function __deliver(tests, status) {
    if (__delivered) {
        return;
    }
    __delivered = true;
    for (var i = 0; i < __callbacks.length; i++) {
        __callbacks[i](tests, status);
    }
}
window.addEventListener("error", function (event) {
    __status = { status: 1, message: String(event.message) };
});
window.addEventListener("load", function () {
    if (__explicit_done) {
        return;
    }
    // Pages driving results through `report(tests, status)` never touch
    // `__tests`, so an empty auto-delivery here would win the runner's
    // first-delivery-wins race and hide their report behind NoResults.
    // Only auto-deliver when `test()` ran or an uncaught error set status.
    if (__tests.length === 0 && __status.status === 0) {
        return;
    }
    __deliver(__tests, __status);
});
