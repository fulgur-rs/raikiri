use crate::runtime::style::{inline_style_value, with_inline_style_property};
use crate::runtime::test_host::StubHost;
use crate::runtime::{DomRuntime, RuntimeError};

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

#[test]
fn inline_style_helpers_match_the_previous_runner_semantics() {
    assert_eq!(
        inline_style_value(Some("color: red; color: blue !important"), "color"),
        "blue"
    );
    assert_eq!(inline_style_value(Some("--x: 1"), "--X"), "");
    assert_eq!(inline_style_value(None, "color"), "");
    assert_eq!(
        with_inline_style_property(Some("color: red; margin: 0"), "COLOR", "blue"),
        "margin: 0; COLOR: blue;"
    );
    assert_eq!(
        with_inline_style_property(Some("color: red;"), "color", " "),
        ""
    );
    assert_eq!(
        with_inline_style_property(Some("color: red;"), "  ", "blue"),
        "color: red;"
    );
}

#[test]
fn style_object_reads_and_writes_camel_and_dashed_names() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s = document.body.style; s.textAlign = 'center'; s.setProperty('word-spacing', '2px');")
        .unwrap();
    ok(&mut rt, "s === document.body.style");
    ok(
        &mut rt,
        "s.getPropertyValue('text-align') === 'center' && s.wordSpacing === '2px'",
    );
    ok(
        &mut rt,
        "s.removeProperty('text-align') === 'center' && s.textAlign === ''",
    );
    ok(
        &mut rt,
        "'getPropertyValue' in s && !('made-up-property' in s)",
    );
    ok(
        &mut rt,
        "var before = document.body.getAttribute('style'); s.setProperty('', 'x'); document.body.getAttribute('style') === before",
    );
}

#[test]
fn style_objects_behave_like_ordinary_objects_for_inherited_members() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(&mut rt, "String(document.body.style) === '[object Object]'");
    ok(&mut rt, "'' + document.body.style === '[object Object]'");
    ok(
        &mut rt,
        "getComputedStyle(document.body).hasOwnProperty('getPropertyValue')",
    );
}

#[test]
fn geometry_reads_flush_only_when_dirty() {
    let (mut host, _, _, body) = StubHost::page();
    host.geometry.insert(body, StubHost::rect(20.4));
    let flushes = host.flushes.clone();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.offsetHeight === 20 && document.body.offsetWidth === 10",
    );
    ok(
        &mut rt,
        "var r = document.body.getBoundingClientRect(); r.x === 1 && r.top === 2 && r.height === 20.4",
    );
    assert_eq!(
        flushes.get(),
        1,
        "initial dirty state flushes once, repeated reads reuse it"
    );
    rt.evaluate("document.body.setAttribute('class', 'z'); document.body.offsetHeight;")
        .unwrap();
    assert_eq!(flushes.get(), 2);
}

#[test]
fn boxless_elements_report_the_zero_rect() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var r = document.head.getBoundingClientRect(); r.width === 0 && r.height === 0 && document.head.offsetHeight === 0",
    );
}

#[test]
fn computed_style_unsupported_property_skips_flush() {
    let (mut host, _, _, body) = StubHost::page();
    host.computed
        .insert((body, "white-space".into()), "normal".into());
    let flushes = host.flushes.clone();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "getComputedStyle(document.body).getPropertyValue('no-such-prop') === ''",
    );
    assert_eq!(flushes.get(), 0);
    ok(
        &mut rt,
        "var cs = getComputedStyle(document.body); cs.whiteSpace === 'normal' && cs.getPropertyValue('white-space') === 'normal' && ('whiteSpace' in cs)",
    );
    assert_eq!(flushes.get(), 1);
    ok(&mut rt, "!('noSuchProp' in cs)");
}

/// `getComputedStyle`'s Proxy traps take any string key, so a dashed CSS
/// property name works through bracket access and `in`, not just the
/// camelCase form `computed_style_unsupported_property_skips_flush` covers.
#[test]
fn computed_style_supports_dashed_property_name_access() {
    let (mut host, _, _, body) = StubHost::page();
    host.computed
        .insert((body, "white-space".into()), "pre".into());
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var cs = getComputedStyle(document.body); \
         ('white-space' in cs) && cs['white-space'] === 'pre'",
    );
}

#[test]
fn get_computed_style_rejects_a_non_element_argument() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let error = rt.evaluate("getComputedStyle(document)").unwrap_err();
    match error {
        RuntimeError::JavaScript(message) => {
            assert!(message.contains("argument is not an Element"), "{message}");
        }
        other => panic!("expected a JavaScript TypeError, got {other:?}"),
    }
}

#[test]
fn computed_value_failure_is_a_host_error() {
    let (mut host, _, _, body) = StubHost::page();
    host.computed
        .insert((body, "white-space".into()), "normal".into());
    host.fail_computed = true;
    let mut rt = DomRuntime::new(host).unwrap();
    assert_eq!(
        rt.evaluate("getComputedStyle(document.body).getPropertyValue('white-space')"),
        Err(RuntimeError::Host("stub computed style failure".into()))
    );
}

#[test]
fn flush_failure_is_a_host_error() {
    let (mut host, ..) = StubHost::page();
    host.fail_flush = true;
    let mut rt = DomRuntime::new(host).unwrap();
    assert_eq!(
        rt.evaluate("document.body.offsetHeight"),
        Err(RuntimeError::Host("stub flush failure".into()))
    );
}

#[test]
fn geometry_failure_is_a_host_error() {
    let (mut host, ..) = StubHost::page();
    host.fail_geometry = true;
    let mut rt = DomRuntime::new(host).unwrap();
    assert_eq!(
        rt.evaluate("document.body.offsetHeight"),
        Err(RuntimeError::Host("stub geometry failure".into()))
    );
}

#[test]
fn style_proxy_traps_cover_symbol_keys_and_computed_writes() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "document.body.style[Symbol.iterator] === undefined",
    );
    ok(
        &mut rt,
        "(document.body.style[Symbol()] = 'x', document.body.getAttribute('style') === null)",
    );
    ok(
        &mut rt,
        "var cs = getComputedStyle(document.body); cs.color = 'red'; document.body.getAttribute('style') === null",
    );
    ok(&mut rt, "!(Symbol() in cs)");
}

#[test]
fn css_supports_uses_the_value_parser() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "CSS.supports('color', 'red') && !CSS.supports('color', '12px') && CSS.supports('color', 'inherit')",
    );
    ok(&mut rt, "!CSS.supports('no-such-property', 'inherit')");
}
