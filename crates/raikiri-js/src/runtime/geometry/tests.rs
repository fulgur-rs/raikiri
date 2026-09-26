use crate::runtime::test_host::StubHost;
use crate::runtime::{DomRuntime, PositionKind, RuntimeError};

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

// ---- CSSOM View offset / client / scroll metrics ---------------------------

/// Append a `tag` element with `id` under `parent` in the stub's document.
fn element(host: &mut StubHost, parent: usize, tag: &str, id: &str) -> usize {
    let index = host.document.create_detached_element(tag).unwrap();
    host.document
        .set_element_attribute(index, "id", id)
        .unwrap();
    host.document.append_child(parent, index).unwrap();
    index
}

fn runtime_over(mut host: StubHost) -> DomRuntime {
    host.document.mark_in_document_flags();
    DomRuntime::new(host).unwrap()
}

#[test]
fn offset_client_and_scroll_metrics_follow_host_geometry() {
    let (mut host, _, _, body) = StubHost::page();
    let rel = element(&mut host, body, "div", "rel");
    let child = element(&mut host, rel, "div", "child");
    element(&mut host, body, "div", "hidden");
    let d = StubHost::dom_rect;
    host.geometry.insert(
        body,
        StubHost::boxed(
            d(0.0, 0.0, 100.0, 100.0),
            d(0.0, 0.0, 100.0, 100.0),
            PositionKind::Static,
        ),
    );
    host.geometry.insert(
        rel,
        StubHost::boxed(
            d(10.0, 20.0, 50.0, 40.0),
            d(12.0, 23.0, 46.0, 34.0),
            PositionKind::Relative,
        ),
    );
    host.geometry.insert(
        child,
        StubHost::boxed(
            d(15.0, 30.0, 10.0, 10.0),
            d(15.0, 30.0, 10.0, 10.0),
            PositionKind::Static,
        ),
    );
    let mut rt = runtime_over(host);
    rt.evaluate(
        "var rel = document.getElementById('rel'), child = document.getElementById('child'), \
         hidden = document.getElementById('hidden');",
    )
    .unwrap();
    ok(
        &mut rt,
        "child.offsetParent === rel && child.offsetTop === 7 && child.offsetLeft === 3",
    );
    ok(
        &mut rt,
        "rel.offsetParent === document.body && rel.offsetTop === 20 && rel.offsetLeft === 10",
    );
    ok(
        &mut rt,
        "rel.clientTop === 3 && rel.clientLeft === 2 && rel.clientWidth === 46 && rel.clientHeight === 34",
    );
    ok(&mut rt, "rel.scrollWidth === 46 && rel.scrollTop === 0");
    ok(
        &mut rt,
        "document.body.offsetParent === null && hidden.offsetParent === null \
         && hidden.offsetTop === 0 && hidden.clientWidth === 0",
    );
}

#[test]
fn metrics_without_a_box_are_zero() {
    let (mut host, _, _, body) = StubHost::page();
    element(&mut host, body, "div", "none");
    let mut rt = runtime_over(host);
    ok(
        &mut rt,
        "var n = document.getElementById('none'); \
         n.offsetTop === 0 && n.offsetLeft === 0 && n.offsetWidth === 0 && n.offsetHeight === 0 \
         && n.clientTop === 0 && n.clientLeft === 0 && n.clientWidth === 0 && n.clientHeight === 0 \
         && n.scrollWidth === 0 && n.scrollHeight === 0 && n.scrollTop === 0 && n.scrollLeft === 0",
    );
}

#[test]
fn body_and_root_have_no_offset_parent_and_body_offsets_are_zero() {
    let (mut host, html, _, body) = StubHost::page();
    let d = StubHost::dom_rect;
    let static_box = |r| StubHost::boxed(r, r, PositionKind::Static);
    host.geometry
        .insert(html, static_box(d(0.0, 0.0, 200.0, 200.0)));
    host.geometry
        .insert(body, static_box(d(8.0, 8.0, 184.0, 184.0)));
    let mut rt = runtime_over(host);
    ok(
        &mut rt,
        "var root = document.documentElement; root.offsetParent === null \
         && root.offsetTop === 0 && root.offsetLeft === 0 && root.offsetWidth === 200",
    );
    ok(
        &mut rt,
        "document.body.offsetParent === null && document.body.offsetTop === 0 \
         && document.body.offsetLeft === 0 && document.body.offsetWidth === 184",
    );
}

#[test]
fn fixed_elements_have_no_offset_parent_and_report_their_own_position() {
    let (mut host, _, _, body) = StubHost::page();
    let rel = element(&mut host, body, "div", "rel");
    let fixed = element(&mut host, rel, "div", "fixed");
    let d = StubHost::dom_rect;
    let r = d(0.0, 0.0, 50.0, 50.0);
    host.geometry
        .insert(rel, StubHost::boxed(r, r, PositionKind::Relative));
    let f = d(4.4, 5.6, 10.0, 10.0);
    host.geometry
        .insert(fixed, StubHost::boxed(f, f, PositionKind::Fixed));
    let mut rt = runtime_over(host);
    ok(
        &mut rt,
        "var f = document.getElementById('fixed'); \
         f.offsetParent === null && f.offsetTop === 6 && f.offsetLeft === 4",
    );
}

#[test]
fn table_cells_are_offset_parents_only_for_static_elements() {
    let (mut host, _, _, body) = StubHost::page();
    let table = element(&mut host, body, "table", "table");
    let tr = element(&mut host, table, "tr", "tr");
    let td = element(&mut host, tr, "td", "td");
    let th = element(&mut host, tr, "th", "th");
    let in_td = element(&mut host, td, "span", "inTd");
    let in_th = element(&mut host, th, "span", "inTh");
    let abs = element(&mut host, td, "span", "abs");
    let d = StubHost::dom_rect;
    let st = |r| StubHost::boxed(r, r, PositionKind::Static);
    for (node, top) in [(table, 1.0), (tr, 2.0), (td, 3.0), (th, 4.0), (in_td, 5.0)] {
        host.geometry.insert(node, st(d(0.0, top, 10.0, 10.0)));
    }
    host.geometry.insert(in_th, st(d(0.0, 9.0, 1.0, 1.0)));
    host.geometry.insert(
        abs,
        StubHost::boxed(
            d(0.0, 6.0, 1.0, 1.0),
            d(0.0, 6.0, 1.0, 1.0),
            PositionKind::Absolute,
        ),
    );
    let mut rt = runtime_over(host);
    ok(
        &mut rt,
        "var g = function (id) { return document.getElementById(id); }; \
         g('inTd').offsetParent === g('td') && g('inTd').offsetTop === 2 \
         && g('inTh').offsetParent === g('th') && g('inTh').offsetTop === 5 \
         && g('tr').offsetParent === g('table') && g('td').offsetParent === g('table') \
         && g('table').offsetParent === document.body \
         && g('abs').offsetParent === document.body && g('abs').offsetTop === 6",
    );
}

#[test]
fn an_element_outside_body_without_a_positioned_ancestor_has_no_offset_parent() {
    let (mut host, html, head, _) = StubHost::page();
    let stray = element(&mut host, head, "div", "stray");
    let d = StubHost::dom_rect;
    let r = d(0.0, 0.0, 300.0, 300.0);
    host.geometry
        .insert(html, StubHost::boxed(r, r, PositionKind::Static));
    let s = d(7.0, 9.0, 3.0, 3.0);
    host.geometry
        .insert(stray, StubHost::boxed(s, s, PositionKind::Static));
    let mut rt = runtime_over(host);
    ok(
        &mut rt,
        "var s = document.getElementById('stray'); \
         s.offsetParent === null && s.offsetTop === 9 && s.offsetLeft === 7",
    );
}

#[test]
fn scroll_extent_and_positions_are_rounded_host_values() {
    let (mut host, _, _, body) = StubHost::page();
    let box_ = element(&mut host, body, "div", "box");
    let d = StubHost::dom_rect;
    let mut g = StubHost::boxed(
        d(0.0, 0.0, 20.0, 20.0),
        d(1.5, 1.5, 17.2, 16.6),
        PositionKind::Sticky,
    );
    g.scroll_width = 40.6;
    g.scroll_height = 70.4;
    host.geometry.insert(box_, g);
    let mut rt = runtime_over(host);
    ok(
        &mut rt,
        "var b = document.getElementById('box'); \
         b.scrollWidth === 41 && b.scrollHeight === 70 \
         && b.clientTop === 2 && b.clientLeft === 2 \
         && b.clientWidth === 17 && b.clientHeight === 17",
    );
}

#[test]
fn scroll_position_setters_convert_their_argument_and_do_nothing() {
    let (mut host, _, _, body) = StubHost::page();
    element(&mut host, body, "div", "box");
    let mut rt = runtime_over(host);
    ok(
        &mut rt,
        "var b = document.getElementById('box'), n = 0; \
         var v = { valueOf: function () { n++; return 5; } }; \
         b.scrollTop = v; b.scrollLeft = v; \
         n === 2 && b.scrollTop === 0 && b.scrollLeft === 0",
    );
    let err = rt.evaluate(
        "document.getElementById('box').scrollTop = { valueOf: function () { throw new TypeError('x'); } };",
    );
    assert!(err.is_err());
}

#[test]
fn metrics_are_element_and_html_element_members() {
    let mut rt = rt();
    ok(
        &mut rt,
        "['clientTop', 'clientLeft', 'clientWidth', 'clientHeight', 'scrollWidth', \
          'scrollHeight', 'scrollTop', 'scrollLeft'].every(function (k) { \
            return Object.getOwnPropertyDescriptor(Element.prototype, k) !== undefined; }) \
         && ['offsetParent', 'offsetTop', 'offsetLeft', 'offsetWidth', 'offsetHeight'] \
            .every(function (k) { \
              return Object.getOwnPropertyDescriptor(HTMLElement.prototype, k) !== undefined; }) \
         && typeof Object.getOwnPropertyDescriptor(Element.prototype, 'scrollTop').set === 'function'",
    );
}

#[test]
fn metric_reads_surface_host_geometry_failures() {
    let (mut host, ..) = StubHost::page();
    host.fail_geometry = true;
    let mut rt = DomRuntime::new(host).unwrap();
    for src in [
        "document.body.offsetParent",
        "document.body.offsetTop",
        "document.body.offsetLeft",
        "document.body.clientTop",
        "document.body.clientLeft",
        "document.body.clientWidth",
        "document.body.clientHeight",
        "document.body.scrollWidth",
        "document.body.scrollHeight",
    ] {
        assert_eq!(
            rt.evaluate(src),
            Err(RuntimeError::Host("stub geometry failure".into())),
            "{src}"
        );
    }
}

#[test]
fn metric_getters_reject_non_elements() {
    let mut rt = rt();
    ok(
        &mut rt,
        "['offsetParent', 'offsetTop', 'offsetLeft', 'offsetWidth', 'offsetHeight'].every(function (k) { \
           var get = Object.getOwnPropertyDescriptor(HTMLElement.prototype, k).get; \
           try { get.call({}); return false; } catch (e) { return e instanceof TypeError; } }) \
         && ['clientTop', 'clientLeft', 'clientWidth', 'clientHeight', 'scrollWidth', \
             'scrollHeight', 'scrollTop', 'scrollLeft'].every(function (k) { \
           var get = Object.getOwnPropertyDescriptor(Element.prototype, k).get; \
           try { get.call({}); return false; } catch (e) { return e instanceof TypeError; } }) \
         && ['scrollTop', 'scrollLeft'].every(function (k) { \
           var set = Object.getOwnPropertyDescriptor(Element.prototype, k).set; \
           try { set.call({}, 1); return false; } catch (e) { return e instanceof TypeError; } })",
    );
}
