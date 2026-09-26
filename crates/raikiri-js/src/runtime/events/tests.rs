use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::{node_index, with_state};
use crate::runtime::{DomRuntime, RuntimeError};

fn rt() -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

fn body_index(rt: &mut DomRuntime) -> usize {
    let value = rt.evaluate("document.body").unwrap();
    node_index(&value).unwrap()
}

fn listener_count(rt: &mut DomRuntime, key: Option<usize>) -> usize {
    with_state(rt.context_mut(), |s| {
        s.listeners.get(&key).map_or(0, Vec::len)
    })
    .unwrap()
}

#[test]
fn add_event_listener_dedupes_by_type_callback_and_capture() {
    let mut rt = rt();
    rt.evaluate(
        "function f(){} \
         document.body.addEventListener('x', f); \
         document.body.addEventListener('x', f); \
         document.body.addEventListener('x', f, true); \
         document.body.addEventListener('x', null); \
         window.addEventListener('load', f);",
    )
    .unwrap();
    let body = body_index(&mut rt);
    assert_eq!(listener_count(&mut rt, Some(body)), 2, "capture false/true");
    assert_eq!(listener_count(&mut rt, None), 1, "window");
}

#[test]
fn remove_event_listener_matches_type_callback_and_capture() {
    let mut rt = rt();
    rt.evaluate(
        "function f(){} \
         document.body.addEventListener('x', f); \
         document.body.addEventListener('x', f, true);",
    )
    .unwrap();
    let body = body_index(&mut rt);
    assert_eq!(listener_count(&mut rt, Some(body)), 2);
    rt.evaluate("document.body.removeEventListener('x', f);")
        .unwrap();
    assert_eq!(listener_count(&mut rt, Some(body)), 1);
    // Removing the same (type, callback) again, now with the wrong capture,
    // does not touch the remaining capture=true entry.
    rt.evaluate("document.body.removeEventListener('x', f);")
        .unwrap();
    assert_eq!(listener_count(&mut rt, Some(body)), 1);
    rt.evaluate("document.body.removeEventListener('x', f, true);")
        .unwrap();
    assert_eq!(listener_count(&mut rt, Some(body)), 0);
}

#[test]
fn remove_event_listener_with_null_callback_is_a_no_op() {
    let mut rt = rt();
    rt.evaluate("function f(){} document.body.addEventListener('x', f);")
        .unwrap();
    let body = body_index(&mut rt);
    rt.evaluate("document.body.removeEventListener('x', null);")
        .unwrap();
    assert_eq!(listener_count(&mut rt, Some(body)), 1);
}

#[test]
fn options_boolean_shorthand_and_dictionary_both_set_capture() {
    let mut rt = rt();
    rt.evaluate(
        "function f(){} function g(){} \
         document.body.addEventListener('x', f, true); \
         document.body.addEventListener('x', g, { capture: true });",
    )
    .unwrap();
    let body = body_index(&mut rt);
    assert_eq!(listener_count(&mut rt, Some(body)), 2);
    rt.evaluate("document.body.removeEventListener('x', f, true); document.body.removeEventListener('x', g, { capture: true });").unwrap();
    assert_eq!(listener_count(&mut rt, Some(body)), 0);
}

#[test]
fn a_non_object_non_boolean_options_value_still_sets_capture_via_to_boolean() {
    let mut rt = rt();
    rt.evaluate("function f(){} document.body.addEventListener('x', f, 1);")
        .unwrap();
    let body = body_index(&mut rt);
    assert_eq!(listener_count(&mut rt, Some(body)), 1);
    // capture=true (from `1`), so removing with the default capture=false
    // does not match.
    rt.evaluate("document.body.removeEventListener('x', f);")
        .unwrap();
    assert_eq!(listener_count(&mut rt, Some(body)), 1);
}

#[test]
fn bare_unqualified_call_targets_the_window() {
    let mut rt = rt();
    rt.evaluate("function f(){} addEventListener('x', f);")
        .unwrap();
    assert_eq!(listener_count(&mut rt, None), 1);
}

#[test]
fn non_object_non_null_callback_is_a_type_error() {
    let mut rt = rt();
    let err = rt.evaluate("document.body.addEventListener('x', 1);");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "{err:?}"
    );
}

#[test]
fn this_not_an_event_target_is_a_type_error() {
    let mut rt = rt();
    let err =
        rt.evaluate("function f(){} EventTarget.prototype.addEventListener.call({}, 'x', f);");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "{err:?}"
    );
}

#[test]
fn event_target_is_illegal_to_construct_directly() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { new EventTarget(); false } catch (e) { e instanceof TypeError }",
    );
}
