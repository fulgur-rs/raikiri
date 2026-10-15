use super::{Position, axis_offset};
use crate::runtime::DomRuntime;
use crate::runtime::host::PositionKind;
use crate::runtime::test_host::StubHost;

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

/// A page whose first body child has a 20px box at (300, 300) in a
/// 100x100 viewport, with `computed` values on the document element.
fn page(position: PositionKind, computed: &[(&str, &str)]) -> DomRuntime {
    let (mut host, html, _, body) = StubHost::page();
    let target = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, target).unwrap();
    let rect = StubHost::dom_rect(300.0, 300.0, 20.0, 20.0);
    host.geometry
        .insert(target, StubHost::boxed(rect, rect, position));
    host.viewport = Ok(Some((100.0, 100.0)));
    for (property, value) in computed {
        host.computed
            .insert((html, (*property).to_owned()), (*value).to_owned());
    }
    DomRuntime::new(host).unwrap()
}

#[test]
fn options_pick_the_alignment_on_each_axis() {
    let mut rt = page(PositionKind::Static, &[]);
    ok(
        &mut rt,
        "var t = document.body.firstChild;\
         function at(arg) { scrollTo(0, 0); t.scrollIntoView(arg); return scrollX + ',' + scrollY; }\
         at() === '220,300' && at(true) === '220,300' && at(false) === '220,220'\
           && at(0) === '220,220' && at(null) === '220,300' && at(undefined) === '220,300'\
           && at({block: 'center', inline: 'center'}) === '260,260'\
           && at({block: 'end', inline: 'start'}) === '300,220'\
           && at({block: 'nearest', inline: 'end', behavior: 'smooth'}) === '220,220'",
    );
}

#[test]
fn dictionary_members_convert_in_order_and_reject_unknown_values() {
    let mut rt = page(PositionKind::Static, &[]);
    ok(
        &mut rt,
        "var t = document.body.firstChild, log = [];\
         var options = {};\
         ['behavior', 'block', 'container', 'inline'].forEach(function (name) {\
           Object.defineProperty(options, name, {get: function () { log.push(name); }});\
         });\
         t.scrollIntoView(options);\
         var rejected = [];\
         function rejects(arg) {\
           t.scrollIntoView(arg).catch(function (e) { rejected.push(e instanceof TypeError); });\
         }\
         rejects({block: 'top'}); rejects({inline: 'left'});\
         rejects({container: 'self'}); rejects({behavior: 'fast'});\
         Element.prototype.scrollIntoView.call({}).catch(function (e) {\
           rejected.push(e instanceof TypeError);\
         });\
         log.join() === 'behavior,block,container,inline'",
    );
    ok(&mut rt, "rejected.join() === 'true,true,true,true,true'");
}

#[test]
fn the_promise_resolves_to_a_non_interrupted_result() {
    let mut rt = page(PositionKind::Static, &[]);
    ok(
        &mut rt,
        "var t = document.body.firstChild, results = [];\
         var scrolled = t.scrollIntoView();\
         var unboxed = document.body.appendChild(document.createElement('p')).scrollIntoView();\
         [scrolled, unboxed].forEach(function (p) {\
           p.then(function (r) { results.push(r.interrupted); });\
         });\
         scrolled instanceof Promise && unboxed instanceof Promise",
    );
    ok(&mut rt, "results.join() === 'false,false'");
}

#[test]
fn an_engine_limit_in_the_options_still_aborts() {
    let mut rt = page(PositionKind::Static, &[]);
    let result = rt.evaluate(
        "var options = {};\
         Object.defineProperty(options, 'block', {get: function f() { return f(); }});\
         document.body.firstChild.scrollIntoView(options)",
    );
    assert!(result.is_err());
}

#[test]
fn elements_without_a_box_or_fixed_do_not_scroll() {
    let mut rt = page(PositionKind::Fixed, &[]);
    ok(
        &mut rt,
        "document.body.firstChild.scrollIntoView();\
         var hidden = document.body.appendChild(document.createElement('p'));\
         hidden.scrollIntoView(); scrollX === 0 && scrollY === 0",
    );
}

#[test]
fn elements_inside_a_fixed_ancestor_do_not_scroll() {
    let (mut host, _, _, body) = StubHost::page();
    let frame = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, frame).unwrap();
    let target = host.document.create_detached_element("div").unwrap();
    host.document.append_child(frame, target).unwrap();
    let rect = StubHost::dom_rect(300.0, 300.0, 20.0, 20.0);
    host.geometry
        .insert(frame, StubHost::boxed(rect, rect, PositionKind::Fixed));
    host.geometry
        .insert(target, StubHost::boxed(rect, rect, PositionKind::Absolute));
    host.viewport = Ok(Some((100.0, 100.0)));
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.firstChild.firstChild.scrollIntoView();\
         scrollX === 0 && scrollY === 0",
    );
}

#[test]
fn the_nearest_container_keeps_the_viewport_inside_a_scroller() {
    let (mut host, html, _, body) = StubHost::page();
    let scroller = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, scroller).unwrap();
    let target = host.document.create_detached_element("div").unwrap();
    host.document.append_child(scroller, target).unwrap();
    let rect = StubHost::dom_rect(300.0, 300.0, 20.0, 20.0);
    host.geometry
        .insert(target, StubHost::boxed(rect, rect, PositionKind::Static));
    host.viewport = Ok(Some((100.0, 100.0)));
    // The body's overflow belongs to the viewport, so it never confines.
    host.computed
        .insert((body, "overflow-y".to_owned()), "hidden".to_owned());
    host.computed
        .insert((html, "overflow-x".to_owned()), "hidden".to_owned());
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var t = document.body.firstChild.firstChild;\
         t.scrollIntoView({container: 'nearest'}); var free = scrollY === 300;\
         scrollTo(0, 0); t.scrollIntoView({container: 'all'});\
         free && scrollY === 300",
    );
    crate::runtime::webidl::with_state(rt.context_mut(), |s| {
        let host = s.host.as_any_mut().downcast_mut::<StubHost>().unwrap();
        host.computed
            .insert((scroller, "overflow-y".to_owned()), "auto".to_owned());
    })
    .unwrap();
    ok(
        &mut rt,
        "scrollTo(0, 0); var t = document.body.firstChild.firstChild;\
         t.scrollIntoView({container: 'nearest'}); var kept = scrollY === 0;\
         t.scrollIntoView(); kept && scrollY === 300",
    );
}

#[test]
fn the_document_writing_mode_decides_the_axes() {
    // Block and inline start, then block and inline end.
    type Computed<'a> = &'a [(&'a str, &'a str)];
    let cases: [(Computed, &str, &str); 5] = [
        (&[("writing-mode", "vertical-rl")], "220,300", "300,220"),
        (&[("writing-mode", "vertical-lr")], "300,300", "220,220"),
        (&[("writing-mode", "sideways-rl")], "220,300", "300,220"),
        (
            &[("writing-mode", "sideways-lr"), ("direction", "rtl")],
            "300,300",
            "220,220",
        ),
        (&[("direction", "rtl")], "220,300", "300,220"),
    ];
    for (computed, expected, ended) in cases {
        let mut rt = page(PositionKind::Static, computed);
        ok(
            &mut rt,
            &format!(
                "var t = document.body.firstChild; t.scrollIntoView({{inline: 'start'}});\
                 var start = scrollX + ',' + scrollY; scrollTo(0, 0);\
                 t.scrollIntoView({{block: 'end', inline: 'end'}});\
                 start === '{expected}' && scrollX + ',' + scrollY === '{ended}'"
            ),
        );
    }
}

#[test]
fn nearest_alignment_moves_the_least() {
    let nearest = |span, offset| axis_offset(Position::Nearest, true, span, offset, 100.0);
    // Already inside, or covering the whole view: no movement.
    assert_eq!(nearest((10.0, 20.0), 0.0), 0.0);
    assert_eq!(nearest((-10.0, 200.0), 0.0), 0.0);
    // Off the low edge: a small span aligns low, a large one high.
    assert_eq!(nearest((-50.0, -40.0), 0.0), -50.0);
    assert_eq!(nearest((-300.0, 50.0), 0.0), -50.0);
    // Off the high edge: a small span aligns high, a large one low.
    assert_eq!(nearest((150.0, 160.0), 0.0), 60.0);
    assert_eq!(nearest((50.0, 400.0), 0.0), 50.0);
}

#[test]
fn a_viewport_failure_fails_the_host_and_no_viewport_has_no_size() {
    let (mut host, _, _, body) = StubHost::page();
    let target = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, target).unwrap();
    let rect = StubHost::dom_rect(300.0, 300.0, 20.0, 20.0);
    host.geometry
        .insert(target, StubHost::boxed(rect, rect, PositionKind::Static));
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.firstChild.scrollIntoView({block: 'end', inline: 'end'});\
         scrollX === 320 && scrollY === 320",
    );
    crate::runtime::webidl::with_state(rt.context_mut(), |s| {
        let host = s.host.as_any_mut().downcast_mut::<StubHost>().unwrap();
        host.viewport = Err(());
    })
    .unwrap();
    assert!(
        rt.evaluate("document.body.firstChild.scrollIntoView()")
            .is_err()
    );
}
