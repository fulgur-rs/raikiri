use raikiri_js::runtime::{DocumentHost, DomRuntime};

use super::WptDocumentHost;
use crate::reftest::{DEFAULT_REFTTEST_HEIGHT, DEFAULT_REFTTEST_WIDTH, prepare_wpt_live_document};

/// Builds a runtime over `html`, returning the backing `TempDir` alongside it
/// so page/font resolution paths captured at setup time stay valid for the
/// runtime's lifetime.
fn runtime(html: &str) -> (tempfile::TempDir, DomRuntime) {
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        html,
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let rt = DomRuntime::new(WptDocumentHost::new(setup, dir.path())).unwrap();
    (dir, rt)
}

fn num(rt: &mut DomRuntime, src: &str) -> f64 {
    rt.evaluate(src).unwrap().as_number().unwrap()
}

fn text(rt: &mut DomRuntime, src: &str) -> String {
    rt.evaluate(src)
        .unwrap()
        .as_string()
        .unwrap()
        .to_std_string_escaped()
}

/// Depth-first search for the element carrying `id`, for tests that need a
/// concrete arena index without going through the runtime's opaque JS handles.
fn find_by_id(document: &raikiri_dom::Document, id: &str) -> usize {
    let mut pending = vec![document.root_index()];
    while let Some(index) = pending.pop() {
        if document.element_attribute(index, "id") == Some(id) {
            return index;
        }
        if let Some(node) = document.get_node(index) {
            pending.extend(node.children.iter().copied());
        }
    }
    panic!("no element with id={id:?} in the test document");
}

#[test]
fn class_change_relayouts_before_geometry_reads() {
    let (_dir, mut rt) =
        runtime("<style>.tall { height: 50px }</style><div id=t style='height:10px'></div>");
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        10.0
    );
    rt.evaluate(
        "var t = document.getElementById('t'); t.removeAttribute('style'); t.classList.add('tall');",
    )
    .unwrap();
    assert_eq!(num(&mut rt, "t.offsetHeight"), 50.0);
    rt.evaluate("t.setAttribute('class', '');").unwrap();
    assert_eq!(num(&mut rt, "t.offsetHeight"), 0.0);
}

#[test]
fn style_inner_html_replaces_sheet_and_template_inner_html_adds_none() {
    let (_dir, mut rt) = runtime(
        "<style id=s>#t { height: 5px }</style><template id=tp></template><div id=t></div>",
    );
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        5.0
    );
    rt.evaluate("document.getElementById('s').innerHTML = '#t { height: 7px }';")
        .unwrap();
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        7.0
    );
    rt.evaluate("document.getElementById('tp').innerHTML = '<style>#t { height: 99px }</style>';")
        .unwrap();
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        7.0
    );
}

/// A `<template>` created by script, not parsed from source, must still get
/// a template-contents fragment root: without one, `innerHTML` on it writes
/// directly to the template element's own children, and a `<style>` in
/// there would be walked by the live-document stylesheet resync and wrongly
/// become an active author stylesheet once the template is connected.
#[test]
fn script_created_template_inner_html_adds_no_stylesheet() {
    let (_dir, mut rt) = runtime("<div id=t></div>");
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        0.0
    );
    rt.evaluate(
        "var tp = document.createElement('template'); document.body.appendChild(tp); \
         tp.innerHTML = '<style>#t { height: 99px }</style>';",
    )
    .unwrap();
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        0.0
    );
}

#[test]
fn detached_style_applies_only_after_connection() {
    let (_dir, mut rt) = runtime("<div id=t></div>");
    rt.evaluate("var s = document.createElement('style'); s.textContent = '#t { height: 12px }';")
        .unwrap();
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        0.0
    );
    rt.evaluate("document.head.appendChild(s);").unwrap();
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        12.0
    );
}

#[test]
fn unsupported_computed_property_does_not_flush() {
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        "<div id=t></div>",
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let host = WptDocumentHost::new(setup, dir.path());
    let flushes = host.flushes.clone();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("getComputedStyle(document.getElementById('t')).getPropertyValue('no-such-prop')")
        .unwrap();
    assert_eq!(flushes.get(), 0);
    rt.evaluate("getComputedStyle(document.getElementById('t')).getPropertyValue('white-space')")
        .unwrap();
    assert_eq!(flushes.get(), 1);
}

/// The removed/added stylesheet diff must also handle a pure removal (an
/// active `<style>` disappearing with nothing new taking its place), not
/// just the replace-in-place case the innerHTML test above exercises.
#[test]
fn removing_a_style_element_from_the_tree_stops_applying_its_rules() {
    let (_dir, mut rt) =
        runtime("<div id=host><style>#t { height: 5px }</style></div><div id=t></div>");
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        5.0
    );
    rt.evaluate("document.getElementById('host').innerHTML = '';")
        .unwrap();
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        0.0
    );
}

/// `computed_value` is documented to be robust against being called before
/// any flush (the runtime always flushes first, but the host must not
/// panic if it doesn't). Only a direct call, bypassing `DomRuntime`'s own
/// flush-before-read wrapper, can reach this path.
#[test]
fn computed_value_before_any_flush_is_a_host_error() {
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        "<div id=t></div>",
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let mut host = WptDocumentHost::new(setup, dir.path());
    assert!(host.computed_value(0, "white-space").is_err());
}

/// Symmetric with the computed-value case above: geometry reads before any
/// flush must also fail cleanly rather than panicking on the absent scene.
#[test]
fn box_geometry_before_any_flush_is_a_host_error() {
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        "<div id=t></div>",
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let mut host = WptDocumentHost::new(setup, dir.path());
    assert!(host.box_geometry(0).is_err());
}

/// `getComputedStyle` on the runtime pre-checks the property name itself and
/// never reaches the host for an unsupported one (see
/// `unsupported_computed_property_does_not_flush` above), so the host's own
/// defensive `None` return for an unsupported name is only reachable through
/// a direct call.
#[test]
fn computed_value_returns_none_for_an_unsupported_property_name() {
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        "<div id=t></div>",
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let mut host = WptDocumentHost::new(setup, dir.path());
    host.flush().unwrap();
    assert_eq!(host.computed_value(0, "no-such-prop").unwrap(), None);
}

/// A `ch`-authored length needs a font's zero-glyph advance to resolve to a
/// concrete pixel value, which only the closure passed to
/// `ComputedProperty::serialize` can measure.
#[test]
fn computed_letter_spacing_in_ch_units_measures_font_advance() {
    let (_dir, mut rt) = runtime("<div id=t style='letter-spacing: 1ch'>x</div>");
    let value = text(
        &mut rt,
        "getComputedStyle(document.getElementById('t')).getPropertyValue('letter-spacing')",
    );
    assert!(
        value.ends_with("px"),
        "expected a measured px length, got {value:?}"
    );
}

/// An element with no generated box (here, `display: none`) reports `None`
/// rather than a zero-sized geometry, distinct from a genuinely zero-height
/// box. `DomRuntime`'s own JS-facing `offsetHeight` collapses both cases to
/// `0`, so only a direct call observes the distinction.
#[test]
fn box_geometry_returns_none_for_an_element_with_no_box() {
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        "<div id=hidden style='display:none'>x</div>",
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let mut host = WptDocumentHost::new(setup, dir.path());
    let hidden = find_by_id(host.document(), "hidden");
    host.flush().unwrap();
    assert_eq!(host.box_geometry(hidden).unwrap(), None);
}

/// CSSOM View metrics over a real layout, checked against CSS 2.1 box-model
/// arithmetic:
///
/// - `o`'s border box is 3 + 5 + 100 + 5 + 3 = 116 wide and
///   3 + 5 + 50 + 5 + 3 = 66 tall; its padding box is 110 x 60 and its
///   border widths (`clientTop`/`clientLeft`) are 3.
/// - `o` is `position: relative`, so it is `i`'s offsetParent. `o`'s
///   border and padding keep `i`'s 7px top margin from collapsing through
///   it (CSS 2.1 §8.3.1), so `i`'s border edge sits 5 + 7 = 12 below `o`'s
///   top padding edge and 5 (the left padding) right of its left one.
/// - `o.offsetTop`/`o.offsetLeft` are both 10, `o`'s own margin, because
///   its offsetParent is the body and the result is relative to the
///   initial containing block. CSS 2.1 with the UA stylesheet's
///   `body { margin: 8px }` gives 10 at the top only through margin
///   collapsing (max(8, 10)) and 8 + 10 = 18 at the left, but this layout
///   places the body box at the origin with the viewport's full size, so
///   the body's margins never reach the geometry (a lone `margin: 5px`
///   block sits at 5, not 8). The measured values are pinned so a layout
///   change there is noticed.
/// - `i` (100 x 200) fits horizontally inside `o`'s 110px padding box, so
///   `scrollWidth` is the padding box width; vertically it ends
///   7 + 200 = 207 below `o`'s top content edge, i.e. 5 + 207 = 212 below
///   the top padding edge. Browsers also add `o`'s bottom padding after
///   in-flow content (217); the scroll extent this host reports is the
///   union of descendant border boxes only.
#[test]
fn cssom_view_metrics_follow_the_css_box_model() {
    let (_dir, mut rt) = runtime(
        "<div id=o style='position:relative; margin:10px; border:3px solid; padding:5px; \
         width:100px; height:50px'><div id=i style='margin-top:7px; height:200px'></div></div>",
    );
    rt.evaluate("var o = document.getElementById('o'), i = document.getElementById('i');")
        .unwrap();
    for (src, expected) in [
        ("i.offsetTop", 12.0),
        ("i.offsetLeft", 5.0),
        ("o.offsetTop", 10.0),
        ("o.offsetLeft", 10.0),
        ("o.offsetWidth", 116.0),
        ("o.offsetHeight", 66.0),
        ("o.clientTop", 3.0),
        ("o.clientLeft", 3.0),
        ("o.clientWidth", 110.0),
        ("o.clientHeight", 60.0),
        ("o.scrollWidth", 110.0),
        ("o.scrollHeight", 212.0),
        ("i.scrollHeight", 200.0),
    ] {
        assert_eq!(num(&mut rt, src), expected, "{src}");
    }
    assert_eq!(
        rt.evaluate("i.offsetParent === o && o.offsetParent === document.body")
            .unwrap()
            .as_boolean(),
        Some(true)
    );
}

/// Every computed `position` keyword the host maps, including the ones
/// (`absolute`, `fixed`, `sticky`) whose layout is not implemented yet: the
/// metrics only need the computed value.
#[test]
fn position_kind_maps_each_computed_position() {
    use raikiri_js::runtime::PositionKind;
    use raikiri_style::property::PositionValue as P;
    for (value, expected) in [
        (P::Static, PositionKind::Static),
        (P::Running("header".into()), PositionKind::Static),
        (P::Relative, PositionKind::Relative),
        (P::Absolute, PositionKind::Absolute),
        (P::Fixed, PositionKind::Fixed),
        (P::Sticky, PositionKind::Sticky),
    ] {
        assert_eq!(super::position_kind(&value), expected, "{value:?}");
    }
}
