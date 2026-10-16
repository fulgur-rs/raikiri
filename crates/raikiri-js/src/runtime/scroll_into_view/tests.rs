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
    page_at(position, computed, (300.0, 300.0))
}

/// `page` with the 20px box at `origin`.
fn page_at(position: PositionKind, computed: &[(&str, &str)], origin: (f64, f64)) -> DomRuntime {
    let (mut host, html, _, body) = StubHost::page();
    let target = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, target).unwrap();
    let rect = StubHost::dom_rect(origin.0, origin.1, 20.0, 20.0);
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
    host.geometry
        .insert(scroller, StubHost::boxed(rect, rect, PositionKind::Static));
    host.geometry
        .insert(body, StubHost::boxed(rect, rect, PositionKind::Static));
    host.viewport = Ok(Some((100.0, 100.0)));
    // With a visible root, the body's overflow belongs to the viewport.
    host.computed
        .insert((body, "overflow-y".to_owned()), "hidden".to_owned());
    host.computed
        .insert((html, "overflow-x".to_owned()), "visible".to_owned());
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
fn the_nearest_container_follows_the_containing_block_chain() {
    let (mut host, _, _, body) = StubHost::page();
    let rect = StubHost::dom_rect(300.0, 300.0, 20.0, 20.0);
    let mut parent = body;
    let mut chain = Vec::new();
    // A relative box, a static scroller, an absolute box, then the target.
    for position in [
        PositionKind::Relative,
        PositionKind::Static,
        PositionKind::Absolute,
        PositionKind::Absolute,
    ] {
        let element = host.document.create_detached_element("div").unwrap();
        host.document.append_child(parent, element).unwrap();
        host.geometry
            .insert(element, StubHost::boxed(rect, rect, position));
        chain.push(element);
        parent = element;
    }
    host.computed
        .insert((chain[1], "overflow-y".to_owned()), "auto".to_owned());
    host.viewport = Ok(Some((100.0, 100.0)));
    let mut rt = DomRuntime::new(host).unwrap();
    let target = "var t = document.body; while (t.firstChild) t = t.firstChild;";
    // The static scroller is not on the absolute boxes' chain.
    ok(
        &mut rt,
        &format!("{target} t.scrollIntoView({{container: 'nearest'}}); scrollY === 300"),
    );
    crate::runtime::webidl::with_state(rt.context_mut(), |s| {
        let host = s.host.as_any_mut().downcast_mut::<StubHost>().unwrap();
        host.computed
            .insert((chain[0], "overflow-x".to_owned()), "hidden".to_owned());
    })
    .unwrap();
    // The relative box contains them, so its overflow confines them.
    ok(
        &mut rt,
        &format!(
            "scrollTo(0, 0); {target} t.scrollIntoView({{container: 'nearest'}}); scrollY === 0"
        ),
    );
}

#[test]
fn an_ancestor_without_a_box_does_not_confine() {
    let (mut host, _, _, body) = StubHost::page();
    let contents = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, contents).unwrap();
    let target = host.document.create_detached_element("div").unwrap();
    host.document.append_child(contents, target).unwrap();
    let rect = StubHost::dom_rect(300.0, 300.0, 20.0, 20.0);
    host.geometry
        .insert(target, StubHost::boxed(rect, rect, PositionKind::Static));
    host.computed
        .insert((contents, "overflow-y".to_owned()), "auto".to_owned());
    host.viewport = Ok(Some((100.0, 100.0)));
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.firstChild.firstChild.scrollIntoView({container: 'nearest'});\
         scrollY === 300",
    );
}

#[test]
fn only_an_html_root_hands_the_body_overflow_to_the_viewport() {
    // A `body` child of a non-HTML document element.
    let (mut host, html, _, _) = StubHost::page();
    let root = host.document.root_index();
    host.document.detach_from_parent(html);
    let svg = host.document.create_detached_element("svg").unwrap();
    host.document.attach_child(root, svg);
    host.document.mark_in_document_flags();
    let body = host.document.create_detached_element("body").unwrap();
    host.document.append_child(svg, body).unwrap();
    let target = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, target).unwrap();
    let rect = StubHost::dom_rect(300.0, 300.0, 20.0, 20.0);
    host.geometry
        .insert(target, StubHost::boxed(rect, rect, PositionKind::Static));
    host.geometry
        .insert(body, StubHost::boxed(rect, rect, PositionKind::Static));
    host.computed
        .insert((body, "overflow-y".to_owned()), "auto".to_owned());
    host.viewport = Ok(Some((100.0, 100.0)));
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.documentElement.firstChild.firstChild.scrollIntoView({container: 'nearest'});\
         scrollY === 0",
    );
}

#[test]
fn foreign_html_and_body_and_inline_boxes_are_not_special() {
    let rect = StubHost::dom_rect(300.0, 300.0, 20.0, 20.0);
    // A foreign-namespace `body` under `html` keeps its own scroller.
    let (mut host, html, _, _) = StubHost::page();
    let body = host.document.create_detached_element("body").unwrap();
    host.document
        .set_element_namespace(body, Some("http://www.w3.org/2000/svg".into()));
    host.document.append_child(html, body).unwrap();
    let target = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, target).unwrap();
    host.geometry
        .insert(target, StubHost::boxed(rect, rect, PositionKind::Static));
    host.geometry
        .insert(body, StubHost::boxed(rect, rect, PositionKind::Static));
    host.computed
        .insert((body, "overflow-y".to_owned()), "auto".to_owned());
    host.viewport = Ok(Some((100.0, 100.0)));
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.documentElement.lastChild.firstChild.scrollIntoView({container: 'nearest'});\
         scrollY === 0",
    );
    // An inline box with `overflow: auto` does not scroll.
    let (mut host, _, _, body) = StubHost::page();
    let span = host.document.create_detached_element("span").unwrap();
    host.document.append_child(body, span).unwrap();
    let target = host.document.create_detached_element("b").unwrap();
    host.document.append_child(span, target).unwrap();
    host.geometry
        .insert(target, StubHost::boxed(rect, rect, PositionKind::Static));
    let mut inline = StubHost::boxed(rect, rect, PositionKind::Static);
    inline.is_inline = true;
    host.geometry.insert(span, inline);
    host.computed
        .insert((span, "overflow-y".to_owned()), "auto".to_owned());
    host.viewport = Ok(Some((100.0, 100.0)));
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.firstChild.firstChild.scrollIntoView({container: 'nearest'});\
         scrollY === 300",
    );
}

#[test]
fn the_body_scrolls_itself_when_the_root_does_not_propagate_it() {
    let (mut host, html, _, body) = StubHost::page();
    let target = host.document.create_detached_element("div").unwrap();
    host.document.append_child(body, target).unwrap();
    let rect = StubHost::dom_rect(300.0, 300.0, 20.0, 20.0);
    host.geometry
        .insert(target, StubHost::boxed(rect, rect, PositionKind::Static));
    host.geometry
        .insert(body, StubHost::boxed(rect, rect, PositionKind::Static));
    host.viewport = Ok(Some((100.0, 100.0)));
    host.computed
        .insert((body, "overflow-y".to_owned()), "auto".to_owned());
    host.computed
        .insert((html, "overflow-x".to_owned()), "visible".to_owned());
    let mut rt = DomRuntime::new(host).unwrap();
    // The root is visible, so the body's overflow is the viewport's.
    ok(
        &mut rt,
        "document.body.firstChild.scrollIntoView({container: 'nearest'}); scrollY === 300",
    );
    crate::runtime::webidl::with_state(rt.context_mut(), |s| {
        let host = s.host.as_any_mut().downcast_mut::<StubHost>().unwrap();
        host.computed
            .insert((html, "overflow-y".to_owned()), "clip".to_owned());
    })
    .unwrap();
    // A clipping root keeps the body's own scrolling box.
    ok(
        &mut rt,
        "scrollTo(0, 0); document.body.firstChild.scrollIntoView({container: 'nearest'});\
         scrollY === 0",
    );
}

#[test]
fn the_document_writing_mode_decides_the_axes() {
    // Block and inline start, then block and inline end. A root whose
    // horizontal axis starts on the right overflows leftward, so its target
    // sits left of the viewport and scrolls to negative offsets.
    type Computed<'a> = &'a [(&'a str, &'a str)];
    let left = (-320.0, 300.0);
    let right = (300.0, 300.0);
    let cases: [(Computed, (f64, f64), &str, &str); 6] = [
        (
            &[("writing-mode", "vertical-rl")],
            left,
            "-400,300",
            "-320,220",
        ),
        (
            &[("writing-mode", "vertical-lr")],
            right,
            "300,300",
            "220,220",
        ),
        (
            &[("writing-mode", "sideways-rl")],
            left,
            "-400,300",
            "-320,220",
        ),
        (
            &[("writing-mode", "sideways-lr"), ("direction", "rtl")],
            right,
            "300,300",
            "220,220",
        ),
        (&[("direction", "rtl")], left, "-400,300", "-320,220"),
        (
            &[("writing-mode", "vertical-lr"), ("direction", "rtl")],
            (300.0, -320.0),
            "300,-400",
            "220,-320",
        ),
    ];
    for (computed, origin, expected, ended) in cases {
        let mut rt = page_at(PositionKind::Static, computed, origin);
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
    // A span exactly as long as the view stays where it is.
    assert_eq!(nearest((-10.0, 90.0), 0.0), 0.0);
    assert_eq!(nearest((10.0, 110.0), 0.0), 0.0);
}

#[test]
fn a_viewport_failure_fails_the_host_and_no_viewport_does_not_scroll() {
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
         scrollX === 0 && scrollY === 0",
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
