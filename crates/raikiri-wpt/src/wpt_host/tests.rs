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
///   `scrollWidth` is the padding box width. Vertically, `i`'s border box
///   ends 5 + 7 + 200 = 212 below `o`'s top padding edge, and scrollable
///   overflow also includes the box's own end-side padding after that
///   content (CSS Overflow 3 §3.3 "Scrollable Overflow"), so
///   `scrollHeight` is 212 + 5 = 217.
/// - `s` is a non-atomic inline box, so its `client*` metrics are all 0
///   (CSSOM View §6) while its border box still has a size.
#[test]
fn cssom_view_metrics_follow_the_css_box_model() {
    let (_dir, mut rt) = runtime(
        "<div id=o style='position:relative; margin:10px; border:3px solid; padding:5px; \
         width:100px; height:50px'><div id=i style='margin-top:7px; height:200px'></div></div>\
         <div><span id=s style='border:2px solid; padding:1px'>x</span></div>",
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
        ("o.scrollHeight", 217.0),
        ("document.getElementById('s').clientTop", 0.0),
        ("document.getElementById('s').clientLeft", 0.0),
        ("document.getElementById('s').clientWidth", 0.0),
        ("document.getElementById('s').clientHeight", 0.0),
        ("i.scrollHeight", 200.0),
    ] {
        assert_eq!(num(&mut rt, src), expected, "{src}");
    }
    assert_eq!(
        rt.evaluate(
            "i.offsetParent === o && o.offsetParent === document.body \
             && document.getElementById('s').offsetWidth > 0"
        )
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

/// A host over an empty document in `page_dir`, with `wpt_root` as the
/// checkout root.
fn host_at(page_dir: &std::path::Path, wpt_root: &std::path::Path) -> WptDocumentHost {
    let setup = prepare_wpt_live_document(
        "",
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        page_dir,
        wpt_root,
    )
    .unwrap();
    WptDocumentHost::new(setup, wpt_root)
}

/// A WPT-root-like temp directory with `resources/helper.js` and
/// `css/page/local.js`, plus the canonical root path.
fn script_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::create_dir_all(root.join("resources")).unwrap();
    std::fs::create_dir_all(root.join("css/page")).unwrap();
    std::fs::write(root.join("resources/helper.js"), "var helper = 1;").unwrap();
    std::fs::write(root.join("css/page/local.js"), "var local = 1;").unwrap();
    (dir, root)
}

fn file_url(path: &std::path::Path) -> String {
    raikiri::Url::from_file_path(path).unwrap().to_string()
}

#[test]
fn document_url_is_the_page_url_or_else_the_base_url() {
    let (_dir, root) = script_root();
    let host = host_at(&root.join("css/page"), &root);
    assert_eq!(
        host.document_url(),
        Some(file_url(&root.join("css/page")) + "/")
    );
    let page = raikiri::Url::from_file_path(root.join("css/page/t.html")).unwrap();
    let host = host.with_page_url(page.clone());
    assert_eq!(host.document_url(), Some(page.to_string()));
}

#[test]
fn fetch_script_reads_a_path_inside_the_root_as_given() {
    let (_dir, root) = script_root();
    let mut host = host_at(&root.join("css/page"), &root);
    let url = file_url(&root.join("css/page/local.js"));
    assert_eq!(host.fetch_script(&url).unwrap(), "var local = 1;");
}

#[test]
fn fetch_script_reroots_a_root_relative_path_at_the_wpt_root() {
    let (_dir, root) = script_root();
    let mut host = host_at(&root.join("css/page"), &root);
    assert_eq!(
        host.fetch_script("file:///resources/helper.js").unwrap(),
        "var helper = 1;"
    );
}

#[test]
fn fetch_script_serves_the_embedded_report_script_without_a_file() {
    let (_dir, root) = script_root();
    let mut host = host_at(&root.join("css/page"), &root);
    let expected = crate::testharness_page::REPORT_SCRIPT;
    assert_eq!(
        host.fetch_script("file:///resources/testharnessreport.js")
            .unwrap(),
        expected
    );
    let as_given = file_url(&root.join("resources/testharnessreport.js"));
    assert_eq!(host.fetch_script(&as_given).unwrap(), expected);
}

#[test]
fn fetch_script_refuses_paths_outside_the_root() {
    let (_dir, root) = script_root();
    let outside = tempfile::tempdir().unwrap();
    let secret = std::fs::canonicalize(outside.path())
        .unwrap()
        .join("secret.js");
    std::fs::write(&secret, "var secret = 1;").unwrap();
    let mut host = host_at(&root.join("css/page"), &root);
    // An absolute path outside the root.
    let error = host.fetch_script(&file_url(&secret)).unwrap_err();
    assert!(
        error.0.contains("no such file inside the WPT root"),
        "{error:?}"
    );
    // `..` segments climbing out of the root from the page's directory.
    let climbing = format!(
        "{}/../../../../../../../../../../..{}",
        file_url(&root.join("css/page")),
        secret.display()
    );
    assert!(host.fetch_script(&climbing).is_err());
    // A symlink inside the root pointing outside it.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&secret, root.join("resources/link.js")).unwrap();
        assert!(
            host.fetch_script("file:///resources/link.js").is_err(),
            "a symlink must not escape the root"
        );
    }
}

#[test]
fn fetch_script_refuses_non_file_and_unusable_urls() {
    let (_dir, root) = script_root();
    let mut host = host_at(&root.join("css/page"), &root);
    for (url, reason) in [
        ("https://example.test/a.js", "only file: URLs are fetched"),
        ("not a url", "not an absolute URL"),
        ("file://remote-host/a.js", "not a local file path"),
    ] {
        let error = host.fetch_script(url).unwrap_err();
        assert!(error.0.contains(reason), "{url}: {error:?}");
    }
}

#[test]
fn fetch_script_reports_read_failures_and_a_missing_root() {
    let (_dir, root) = script_root();
    let mut host = host_at(&root.join("css/page"), &root);
    let error = host.fetch_script("file:///resources/").unwrap_err();
    assert!(error.0.contains("read failed"), "{error:?}");
    std::fs::write(root.join("resources/latin1.js"), [0xff, 0xfe]).unwrap();
    let error = host
        .fetch_script("file:///resources/latin1.js")
        .unwrap_err();
    assert!(error.0.contains("read failed"), "{error:?}");

    let missing = root.join("no-such-root");
    let mut host = host_at(&root.join("css/page"), &missing);
    let error = host
        .fetch_script("file:///resources/helper.js")
        .unwrap_err();
    assert!(error.0.contains("the WPT root does not exist"), "{error:?}");
}

#[test]
fn scroll_content_extents_match_a_brute_force_walk() {
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        "<div id=o style='position:relative; border:3px solid; padding:5px; width:100px; height:50px'>\
         <div id=i style='margin-top:7px; height:200px'></div>\
         <div id=hidden style='display:none'><div id=inner style='height:99px'></div></div></div>",
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let mut host = WptDocumentHost::new(setup, dir.path());
    host.flush().unwrap();
    let count = host.document().node_count();
    assert_eq!(host.scroll_content_extents.len(), count);
    for node in 0..count {
        let (expected_right, expected_bottom) = {
            let scene = host.page_scene.as_ref().unwrap();
            let document = host.document();
            let (mut expected_right, mut expected_bottom) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            let mut pending: Vec<usize> = document
                .get_node(node)
                .map(|n| n.children.clone())
                .unwrap_or_default();
            while let Some(index) = pending.pop() {
                if let Some(descendant) = super::border_box_of(scene, index) {
                    expected_right = expected_right.max(descendant.right);
                    expected_bottom = expected_bottom.max(descendant.bottom);
                }
                if let Some(n) = document.get_node(index) {
                    pending.extend(n.children.iter().copied());
                }
            }
            (expected_right, expected_bottom)
        };
        let (cached_right, cached_bottom) = host.scroll_content_extents[node];
        assert_eq!(cached_right, expected_right, "right content of node {node}");
        assert_eq!(
            cached_bottom, expected_bottom,
            "bottom content of node {node}"
        );
        let geometry = host.box_geometry(node).unwrap();
        if let Some(geometry) = geometry {
            let padding = host
                .document()
                .get_node(node)
                .map(|n| n.unrounded_layout.padding)
                .unwrap_or_default();
            let right = geometry
                .padding_box
                .right
                .max(expected_right + f64::from(padding.right));
            let bottom = geometry
                .padding_box
                .bottom
                .max(expected_bottom + f64::from(padding.bottom));
            assert_eq!(
                geometry.scroll_width,
                right - geometry.padding_box.left,
                "scroll width of node {node}"
            );
            assert_eq!(
                geometry.scroll_height,
                bottom - geometry.padding_box.top,
                "scroll height of node {node}"
            );
        }
    }
}

#[test]
fn scroll_extents_are_cached_per_flush_and_invalidated_by_flush() {
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        "<div id=o style='width:100px; height:50px'><div id=i style='height:200px'></div></div>",
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let mut host = WptDocumentHost::new(setup, dir.path());
    host.flush().unwrap();
    let before = host.scroll_content_extents.clone();
    assert_eq!(before.len(), host.document().node_count());
    let o = find_by_id(host.document(), "o");
    let first = host.box_geometry(o).unwrap().unwrap();
    for _ in 0..10 {
        let geometry = host.box_geometry(o).unwrap().unwrap();
        assert_eq!(geometry.scroll_width, first.scroll_width);
        assert_eq!(geometry.scroll_height, first.scroll_height);
    }
    assert_eq!(
        host.scroll_content_extents, before,
        "geometry reads must not recompute the cache"
    );
    let tall = host.document_mut().create_detached_element("div").unwrap();
    host.document_mut()
        .set_element_attribute(tall, "style", "height:400px")
        .unwrap();
    host.document_mut().append_child(o, tall).unwrap();
    host.flush().unwrap();
    let after = host.box_geometry(o).unwrap().unwrap();
    assert!(
        after.scroll_height > first.scroll_height,
        "expected growth, first {} then {}",
        first.scroll_height,
        after.scroll_height
    );
    assert_ne!(
        host.scroll_content_extents, before,
        "a flush must recompute the cache"
    );
}

#[test]
fn deep_offset_top_reads_through_cached_extents() {
    const DEPTH: usize = 150;
    let mut html = String::from("<div id=o0 style='position:relative'>");
    for i in 1..DEPTH {
        html.push_str(&format!("<div id=o{i}>"));
    }
    html.push_str("<div id=deep style='height:5px'>x</div>");
    for _ in 1..DEPTH {
        html.push_str("</div>");
    }
    html.push_str("</div>");
    let dir = tempfile::tempdir().unwrap();
    let setup = prepare_wpt_live_document(
        &html,
        DEFAULT_REFTTEST_WIDTH,
        DEFAULT_REFTTEST_HEIGHT,
        dir.path(),
        dir.path(),
    )
    .unwrap();
    let host = WptDocumentHost::new(setup, dir.path());
    let flushes = host.flushes.clone();
    let mut rt = DomRuntime::new(host).unwrap();
    let top = num(&mut rt, "document.getElementById('deep').offsetTop");
    assert!(top.is_finite(), "deep offsetTop did not resolve");
    assert_eq!(
        rt.evaluate(
            "document.getElementById('deep').offsetParent === document.getElementById('o0')"
        )
        .unwrap()
        .as_boolean(),
        Some(true)
    );
    assert_eq!(
        flushes.get(),
        1,
        "deep read must flush once, not per ancestor"
    );
}

/// An `innerHTML`-inserted `<style>` keeps the fragment parser's `@import`
/// expansion when the host reconciles at flush. Without expansion the import
/// stays opaque and the probe keeps height 0.
#[test]
fn inner_html_style_expands_leading_import() {
    let (dir, mut rt) = runtime("<div id=t></div>");
    std::fs::write(dir.path().join("imported.css"), "#t { height: 33px }").unwrap();
    rt.evaluate(r#"document.head.innerHTML = '<style>@import "imported.css";</style>';"#)
        .unwrap();
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        33.0
    );
}

/// An SVG-namespace `<style>` connected by script reaches the cascade like the
/// fragment parser's `stylesheet_sources` projection, which keeps SVG styles.
#[test]
fn svg_namespace_style_connected_by_script_applies() {
    let (_dir, mut rt) = runtime("<div id=t></div>");
    rt.evaluate(
        "var s = document.createElementNS('http://www.w3.org/2000/svg', 'style');          s.textContent = '#t { height: 21px }'; document.body.appendChild(s);",
    )
    .unwrap();
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        21.0
    );
}

/// Author order follows tree order. A script-connected `<style>` inserted
/// before an existing sheet loses to it; appending the same buggy behavior
/// at the end of the author list would let the earlier sheet win instead.
#[test]
fn inserted_style_before_existing_sheet_follows_tree_order() {
    let (_dir, mut rt) = runtime("<style id=a>#t { height: 5px }</style><div id=t></div>");
    rt.evaluate(
        "var s = document.createElement('style'); s.textContent = '#t { height: 12px }';          var a = document.getElementById('a'); a.parentNode.insertBefore(s, a);",
    )
    .unwrap();
    assert_eq!(
        num(&mut rt, "document.getElementById('t').offsetHeight"),
        5.0
    );
}

/// The `DocumentHost` downcast hooks recover the concrete WPT host from a
/// runtime or a boxed trait object, with its mutated document.
#[test]
fn concrete_wpt_host_recovers_from_the_runtime() {
    use raikiri_js::runtime::DocumentHost;

    let (_dir, mut rt) = runtime("<div id=t></div>");
    rt.evaluate("document.getElementById('t').textContent = 'hi';")
        .unwrap();
    let host: WptDocumentHost = rt
        .try_into_host::<WptDocumentHost>()
        .unwrap_or_else(|_| panic!("expected the runtime to hold a WptDocumentHost"));
    let found = find_by_id(host.document(), "t");
    assert_eq!(
        host.document().element_text_content(found).as_deref(),
        Some("hi")
    );
    let (_dir, rt) = runtime("<div></div>");
    let mut boxed = rt.into_host();
    assert!(boxed.downcast_ref::<WptDocumentHost>().is_some());
    assert!(boxed.downcast_mut::<WptDocumentHost>().is_some());
    assert!(boxed.downcast::<WptDocumentHost>().is_ok());
}
