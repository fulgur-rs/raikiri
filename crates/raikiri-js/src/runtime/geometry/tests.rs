use crate::runtime::test_host::StubHost;
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

#[test]
fn negative_width_and_height_still_give_a_self_consistent_box() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var r = new DOMRect(1, 2, -3, 4); \
         r.left === -2 && r.right === 1 && r.top === 2 && r.bottom === 6 \
         && r instanceof DOMRectReadOnly && r instanceof DOMRect",
    );
}

#[test]
fn to_json_reports_all_eight_fields() {
    let mut rt = rt();
    rt.evaluate("var r = new DOMRect(1, 2, -3, 4);").unwrap();
    ok(
        &mut rt,
        "var j = r.toJSON(); j.x === 1 && j.y === 2 && j.width === -3 && j.height === 4 \
         && j.top === 2 && j.left === -2 && j.right === 1 && j.bottom === 6 \
         && Object.keys(j).length === 8",
    );
}

#[test]
fn get_bounding_client_rect_returns_a_dom_rect() {
    let mut rt = rt();
    ok(
        &mut rt,
        "document.body.getBoundingClientRect() instanceof DOMRect",
    );
}

#[test]
fn missing_arguments_default_to_zero_and_explicit_undefined_matches() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var r = new DOMRect(); r.x === 0 && r.y === 0 && r.width === 0 && r.height === 0",
    );
    ok(
        &mut rt,
        "var r2 = new DOMRect(undefined, undefined, undefined, undefined); \
         r2.x === 0 && r2.y === 0 && r2.width === 0 && r2.height === 0",
    );
}

#[test]
fn dom_rect_is_writable_and_dom_rect_read_only_is_not_a_setter() {
    let mut rt = rt();
    rt.evaluate("var r = new DOMRect(1, 2, 3, 4); r.x = 9; r.y = 8; r.width = 7; r.height = 6;")
        .unwrap();
    ok(
        &mut rt,
        "r.x === 9 && r.y === 8 && r.width === 7 && r.height === 6 \
         && r.left === 9 && r.right === 16 && r.top === 8 && r.bottom === 14",
    );
    // `DOMRectReadOnly` has no setter for `x`; a sloppy-mode assignment
    // through an accessor with only a getter is silently ignored.
    rt.evaluate("var ro = new DOMRectReadOnly(1, 2, 3, 4); ro.x = 100;")
        .unwrap();
    ok(&mut rt, "ro.x === 1");
}

#[test]
fn nan_propagates_through_top_right_bottom_left() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var r = new DOMRect(1, 2, NaN, 4); \
         Number.isNaN(r.right) && Number.isNaN(r.left) && r.top === 2 && r.bottom === 6",
    );
}

#[test]
fn constructors_require_new() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { DOMRect(1, 2, 3, 4); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { DOMRectReadOnly(1, 2, 3, 4); false } catch (e) { e instanceof TypeError }",
    );
}

#[test]
fn brand_mismatch_on_a_getter_is_a_type_error_not_a_panic() {
    let mut rt = rt();
    let err = rt.evaluate("Object.getOwnPropertyDescriptor(DOMRect.prototype, 'x').get.call({})");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "{err:?}"
    );
}
