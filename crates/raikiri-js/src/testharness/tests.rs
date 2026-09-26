use super::*;
use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

/// Missing and empty-valued attributes must be distinguishable through
/// `getAttribute`/`hasAttribute`, not collapsed to the same observable state.
#[test]
fn element_attribute_reads_distinguish_missing_and_empty_values() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        r#"
            var line = document.createElement('div');
            document.body.appendChild(line);
            line.setAttribute('data-empty', '');
            line.setAttribute('title', 'initial');
        "#,
    )
    .unwrap();
    ok(
        &mut rt,
        "line.getAttribute('missing') === null && !line.hasAttribute('missing')",
    );
    ok(
        &mut rt,
        "line.getAttribute('data-empty') === '' && line.hasAttribute('data-empty')",
    );
    ok(
        &mut rt,
        "line.getAttribute('title') === 'initial' && line.hasAttribute('title')",
    );
}

/// `setAttribute` coerces a non-string argument (DOM §4.9's `DOMString`
/// argument type) and `removeAttribute` makes a later read report the
/// attribute as absent again.
#[test]
fn element_attribute_mutations_round_trip_through_native_bindings() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var line = document.createElement('div'); document.body.appendChild(line);")
        .unwrap();
    rt.evaluate("line.setAttribute('data-count', 42);").unwrap();
    ok(
        &mut rt,
        "line.getAttribute('data-count') === '42' && line.hasAttribute('data-count')",
    );
    rt.evaluate("line.removeAttribute('data-count');").unwrap();
    ok(
        &mut rt,
        "line.getAttribute('data-count') === null && !line.hasAttribute('data-count')",
    );
}

#[test]
fn assert_not_equals_checks_inequality_and_reports_equal_values() {
    let (host, ..) = StubHost::page();
    let passed = run_testharness_on_host(
        "test(function() { assert_not_equals('a', 'b'); }, 'different values');",
        host,
    )
    .unwrap();
    assert_eq!(passed.len(), 1);
    assert!(passed[0].passed, "{:?}", passed[0]);

    let (host, ..) = StubHost::page();
    let failed = run_testharness_on_host(
        "test(function() { assert_not_equals('a', 'a'); }, 'equal values');",
        host,
    )
    .unwrap();
    assert_eq!(failed.len(), 1);
    assert!(!failed[0].passed);
    assert!(failed[0].message.contains("assert_not_equals"));
}

#[test]
fn assert_in_array_checks_membership_and_reports_non_members() {
    let (host, ..) = StubHost::page();
    let passed = run_testharness_on_host(
        "test(function() { assert_in_array('b', ['a', 'b', 'c']); }, 'member');",
        host,
    )
    .unwrap();
    assert_eq!(passed.len(), 1);
    assert!(passed[0].passed, "{:?}", passed[0]);

    let (host, ..) = StubHost::page();
    let failed = run_testharness_on_host(
        "test(function() { assert_in_array('z', ['a', 'b', 'c']); }, 'non-member');",
        host,
    )
    .unwrap();
    assert_eq!(failed.len(), 1);
    assert!(!failed[0].passed);
    assert!(failed[0].message.contains("assert_in_array"));
}

#[test]
fn empty_script_is_reported_as_no_tests() {
    let (host, ..) = StubHost::page();
    let result = run_testharness_on_host("", host);
    assert!(matches!(result, Err(TestHarnessError::NoTests)));
}

/// A `test()` body that throws (here, a property read on `null`) is reported
/// as a failed outcome, not propagated as an uncaught engine error.
#[test]
fn missing_element_is_an_assertion_failure_not_a_pass() {
    let (host, ..) = StubHost::page();
    let result = run_testharness_on_host(
        "test(function() { assert_true(document.getElementById('missing').offsetHeight > 35); }, 'missing');",
        host,
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert!(!result[0].passed);
    assert!(result[0].message.contains("null"));
}

#[test]
fn callbacks_that_requeue_themselves_hit_the_event_loop_limit() {
    let (host, ..) = StubHost::page();
    let result = run_testharness_on_host(
        "document.fonts.ready.then(function repeat() { document.fonts.ready.then(repeat); }); test(function() { assert_true(true); }, 'initial');",
        host,
    );
    assert!(matches!(result, Err(TestHarnessError::EventLoopLimit)));
}

#[test]
fn font_load_callbacks_run_through_the_testharness_event_loop() {
    let (host, ..) = StubHost::page();
    let result = run_testharness_on_host(
        r#"document.fonts.load("20px Ahem").then(function(fonts) {
            test(function() { assert_equals(fonts.length, 0); }, "font load callback");
        });"#,
        host,
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].passed, "{:?}", result[0]);
}

/// A callback deferred behind `document.fonts.ready` still observes DOM
/// state set up before it fires (here, a configured element and its
/// geometry), and can safely mutate the tree from inside its own `test()`.
#[test]
fn geometry_and_parent_mutations_are_visible_to_a_deferred_font_callback() {
    let (mut host, _, _, body) = StubHost::page();
    let span = host.document.create_detached_element("span").unwrap();
    host.document
        .set_element_attribute(span, "id", "span")
        .unwrap();
    host.document.append_child(body, span).unwrap();
    host.document.mark_in_document_flags();
    host.geometry.insert(span, StubHost::rect(30.0));

    let result = run_testharness_on_host(
        r#"
            setup({explicit_done: true});
            document.fonts.ready.then(function() {
                test(function() {
                    assert_approx_equals(document.getElementById('span').getBoundingClientRect().left, 1, 1);
                    document.getElementById('span').parentNode.style.display = 'none';
                }, 'left');
                done();
            });
        "#,
        host,
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].passed, "{:?}", result[0]);
}

#[test]
fn run_testharness_on_host_reports_outcomes_from_native_dom() {
    let (host, ..) = crate::runtime::test_host::StubHost::page();
    let outcomes = crate::testharness::run_testharness_on_host(
        "test(function () { assert_equals(document.body.tagName, 'BODY'); }, 'body');
         test(function () { assert_true(document.body instanceof HTMLElement); }, 'proto');",
        host,
    )
    .unwrap();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|o| o.passed), "{outcomes:?}");
}

#[test]
fn run_testharness_on_host_maps_host_failures_to_dom_errors() {
    let (mut host, ..) = crate::runtime::test_host::StubHost::page();
    host.fail_flush = true;
    let result = crate::testharness::run_testharness_on_host(
        "test(function () { document.body.offsetHeight; }, 'geometry');",
        host,
    );
    assert!(
        matches!(result, Err(crate::testharness::TestHarnessError::Dom(ref m)) if m == "stub flush failure"),
        "{result:?}"
    );
}

#[test]
fn run_testharness_on_host_maps_uncaught_js_errors_to_javascript_errors() {
    let (host, ..) = crate::runtime::test_host::StubHost::page();
    let result = crate::testharness::run_testharness_on_host("throw new Error('uncaught');", host);
    assert!(
        matches!(
            result,
            Err(crate::testharness::TestHarnessError::JavaScript(_))
        ),
        "{result:?}"
    );
}

/// A script that reassigns the font-callback-queue shim global to a
/// non-object must not panic the process; it should surface as an ordinary
/// harness error instead.
#[test]
fn reassigning_the_font_callback_queue_is_a_javascript_error_not_a_panic() {
    let (host, ..) = StubHost::page();
    let result = run_testharness_on_host("__raikiri_font_callbacks = 1;", host);
    assert!(
        matches!(result, Err(TestHarnessError::JavaScript(_))),
        "{result:?}"
    );
}

/// Same as above, but for a non-object entry pushed into the results queue
/// (the queue itself is still a well-formed array).
#[test]
fn pushing_a_non_object_test_result_is_a_javascript_error_not_a_panic() {
    let (host, ..) = StubHost::page();
    let result = run_testharness_on_host("__raikiri_results.push(1);", host);
    assert!(
        matches!(result, Err(TestHarnessError::JavaScript(_))),
        "{result:?}"
    );
}

/// Same as above, but the results queue global itself is reassigned to a
/// non-object.
#[test]
fn reassigning_the_results_queue_is_a_javascript_error_not_a_panic() {
    let (host, ..) = StubHost::page();
    let result = run_testharness_on_host("__raikiri_results = 5;", host);
    assert!(
        matches!(result, Err(TestHarnessError::JavaScript(_))),
        "{result:?}"
    );
}
