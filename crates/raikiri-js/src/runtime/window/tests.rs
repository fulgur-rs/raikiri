use std::any::Any;

use boa_engine::property::Attribute;
use boa_engine::{JsString, JsValue};
use raikiri_dom::Document;

use crate::runtime::DomRuntime;
use crate::runtime::host::{BoxGeometry, DocumentHost, HostError};
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;

fn rt() -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

fn rt_with_url(url: &str) -> DomRuntime {
    let (mut host, ..) = StubHost::page();
    host.document_url = Some(url.to_owned());
    DomRuntime::new(host).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

/// A [`DocumentHost`] that delegates everything to a [`StubHost`] except
/// `document_url`, which it leaves at the trait's own default (`None`):
/// `StubHost` overrides that method (so tests can set a URL), which means
/// nothing else in this crate ever exercises the default body itself.
struct DefaultUrlHost(StubHost);

impl DocumentHost for DefaultUrlHost {
    fn document(&self) -> &Document {
        self.0.document()
    }
    fn document_mut(&mut self) -> &mut Document {
        self.0.document_mut()
    }
    fn flush(&mut self) -> Result<(), HostError> {
        self.0.flush()
    }
    fn box_geometry(&mut self, node: usize) -> Result<Option<BoxGeometry>, HostError> {
        self.0.box_geometry(node)
    }
    fn computed_value(&mut self, node: usize, property: &str) -> Result<Option<String>, HostError> {
        self.0.computed_value(node, property)
    }
    fn parse_fragment(
        &mut self,
        context_tag: &str,
        context_ns: &str,
        markup: &str,
    ) -> Result<Document, HostError> {
        self.0.parse_fragment(context_tag, context_ns, markup)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

#[test]
fn document_url_default_trait_body_is_none() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(DefaultUrlHost(host)).unwrap();
    ok(
        &mut rt,
        "location.href === 'about:blank' && document.URL === 'about:blank'",
    );
}

#[test]
fn location_reports_every_component_of_an_explicit_url() {
    let mut rt = rt_with_url("https://example.test:8443/a/b.html?q=1#h");
    ok(
        &mut rt,
        "location.href === 'https://example.test:8443/a/b.html?q=1#h' \
         && location.protocol === 'https:' \
         && location.host === 'example.test:8443' \
         && location.hostname === 'example.test' \
         && location.port === '8443' \
         && location.pathname === '/a/b.html' \
         && location.search === '?q=1' \
         && location.hash === '#h' \
         && location.origin === 'https://example.test:8443' \
         && location.toString() === location.href",
    );
}

#[test]
fn location_reports_a_url_with_no_explicit_port_or_query_or_fragment() {
    let mut rt = rt_with_url("http://example.test/a");
    ok(
        &mut rt,
        "location.href === 'http://example.test/a' \
         && location.protocol === 'http:' \
         && location.host === 'example.test' \
         && location.hostname === 'example.test' \
         && location.port === '' \
         && location.pathname === '/a' \
         && location.search === '' \
         && location.hash === '' \
         && location.origin === 'http://example.test'",
    );
}

#[test]
fn location_reports_a_bare_delimiter_query_or_fragment_as_empty() {
    // `Location.search`/`.hash` (WHATWG URL Standard): a trailing `?`/`#`
    // with nothing after it reads back as `""`, not the bare delimiter.
    let mut rt = rt_with_url("https://example.test/a?");
    ok(&mut rt, "location.search === '' && location.hash === ''");
    let mut rt = rt_with_url("https://example.test/a#");
    ok(&mut rt, "location.search === '' && location.hash === ''");
}

#[test]
fn url_search_params_returns_first_and_all_duplicate_values_in_order() {
    let mut rt = rt();
    ok(
        &mut rt,
        "const params = new URLSearchParams('?class=halt,htb&class=chws'); \
         const classes = params.getAll('class'); \
         params.has('class') \
         && params.get('class') === 'halt,htb' \
         && classes.join('|') === 'halt,htb|chws' \
         && classes.flatMap(value => value.split(',')).join('|') === 'halt|htb|chws'",
    );
}

#[test]
fn url_search_params_decodes_form_encoded_values() {
    let mut rt = rt();
    ok(
        &mut rt,
        "const params = new URLSearchParams('?value=space+with%20plus&city=%E6%9D%B1%E4%BA%AC'); \
         params.get('value') === 'space with plus' \
         && params.get('city') === '東京'",
    );
}

#[test]
fn url_search_params_empty_query_and_missing_names_have_empty_results() {
    let mut rt = rt();
    ok(
        &mut rt,
        "const params = new URLSearchParams(''); \
         !params.has('missing') \
         && params.get('missing') === null \
         && params.getAll('missing').length === 0",
    );
}

#[test]
fn url_search_params_rejects_calls_with_invalid_brand_or_missing_name() {
    let mut rt = rt();
    let error = rt
        .evaluate("URLSearchParams.prototype.get.call({}, 'missing')")
        .unwrap_err();
    assert!(error.to_string().contains("not a URLSearchParams"));

    let error = rt.evaluate("new URLSearchParams().get()").unwrap_err();
    assert!(error.to_string().contains("name is required"));
}

#[test]
fn url_search_params_constructor_requires_new_and_an_object_prototype() {
    let mut rt = rt();
    let error = rt.evaluate("URLSearchParams('?x=1')").unwrap_err();
    assert!(error.to_string().contains("requires 'new'"));

    let error = rt
        .evaluate(
            "function InvalidPrototype() {} \
             InvalidPrototype.prototype = null; \
             Reflect.construct(URLSearchParams, [], InvalidPrototype)",
        )
        .unwrap_err();
    assert!(error.to_string().contains("prototype is not an object"));
}

#[test]
fn url_search_params_install_propagates_global_registration_errors() {
    let mut rt = rt();
    rt.context_mut()
        .register_global_property(
            JsString::from("URLSearchParams"),
            JsValue::null(),
            Attribute::empty(),
        )
        .unwrap();

    let result = super::super::url_search_params::install(rt.context_mut());

    assert!(result.is_err());
}

#[test]
fn location_defaults_to_about_blank_without_a_document_url() {
    let mut rt = rt();
    ok(
        &mut rt,
        "location.href === 'about:blank' \
         && location.protocol === 'about:' \
         && location.host === '' \
         && location.hostname === '' \
         && location.port === '' \
         && location.pathname === 'blank' \
         && location.search === '' \
         && location.hash === '' \
         && location.origin === 'null' \
         && location.toString() === 'about:blank'",
    );
}

#[test]
fn location_setters_and_navigation_methods_are_no_ops() {
    let mut rt = rt_with_url("https://example.test:8443/a/b.html?q=1#h");
    rt.evaluate(
        "location.href = 'https://other.test/'; \
         location.protocol = 'http:'; \
         location.host = 'other.test'; \
         location.hostname = 'other.test'; \
         location.port = '80'; \
         location.pathname = '/x'; \
         location.search = '?z'; \
         location.hash = '#z'; \
         location.assign('https://other.test/'); \
         location.replace('https://other.test/'); \
         location.reload();",
    )
    .unwrap();
    ok(
        &mut rt,
        "location.href === 'https://example.test:8443/a/b.html?q=1#h'",
    );
}

#[test]
fn window_parent_top_frames_and_opener() {
    let mut rt = rt();
    ok(
        &mut rt,
        "window.parent === window && window.top === window && window.frames === window \
         && window.opener === null",
    );
}

#[test]
fn location_setter_forwards_to_href_and_does_not_replace_the_singleton() {
    let mut rt = rt();
    // `location`'s setter is a `[PutForwards=href]`-style forward (a no-op
    // here, since navigation is out of scope): assignment never replaces
    // the singleton object itself, unlike a plain writable data property.
    rt.evaluate("location = 'https://other.test/';").unwrap();
    ok(
        &mut rt,
        "typeof location === 'object' && location.href === 'about:blank'",
    );
}

#[test]
fn navigator_console_parent_frames_are_replaceable() {
    let mut rt = rt();
    // `[Replaceable]` (approximated as a plain writable data property):
    // assignment actually replaces the value, unlike `location`/`top`.
    rt.evaluate("navigator = 1; console = 2; parent = 3; frames = 4;")
        .unwrap();
    ok(
        &mut rt,
        "navigator === 1 && console === 2 && parent === 3 && frames === 4",
    );
}

#[test]
fn top_stays_unforgeable_unlike_parent_and_frames() {
    let mut rt = rt();
    // `top` is `[LegacyUnforgeable]`, not `[Replaceable]` like `parent`/
    // `frames`: non-strict assignment to it is a silent no-op.
    rt.evaluate("top = 1;").unwrap();
    ok(&mut rt, "top === window");
}

#[test]
fn location_and_opener_assignment_do_not_throw_in_strict_mode() {
    let mut rt = rt();
    ok(
        &mut rt,
        "(function () { \
           'use strict'; \
           window.location = 'https://other.test/'; \
           window.opener = null; \
           return true; \
         })()",
    );
}

#[test]
fn navigator_user_agent_is_a_fixed_string() {
    let mut rt = rt();
    ok(
        &mut rt,
        "navigator.userAgent === 'Mozilla/5.0 (compatible; raikiri)'",
    );
}

#[test]
fn console_methods_record_level_and_joined_message_in_order() {
    let mut rt = rt();
    rt.evaluate(
        "console.log('a', 1); \
         console.error('b'); \
         console.warn('c', 'd'); \
         console.info(); \
         console.debug('e');",
    )
    .unwrap();
    let messages = with_state(rt.context_mut(), |s| s.console.clone()).unwrap();
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0].level, super::ConsoleLevel::Log);
    assert_eq!(messages[0].message, "a 1");
    assert_eq!(messages[1].level, super::ConsoleLevel::Error);
    assert_eq!(messages[1].message, "b");
    assert_eq!(messages[2].level, super::ConsoleLevel::Warn);
    assert_eq!(messages[2].message, "c d");
    assert_eq!(messages[3].level, super::ConsoleLevel::Info);
    assert_eq!(messages[3].message, "");
    assert_eq!(messages[4].level, super::ConsoleLevel::Debug);
    assert_eq!(messages[4].message, "e");
}

#[test]
fn console_argument_tostring_exception_records_nothing_and_propagates() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { console.log({ toString() { throw new Error('boom'); } }); false } \
         catch (e) { e.message === 'boom' }",
    );
    let messages = with_state(rt.context_mut(), |s| s.console.clone()).unwrap();
    assert!(messages.is_empty());
}

#[test]
fn location_and_navigator_have_tostringtag() {
    let mut rt = rt();
    ok(
        &mut rt,
        "Object.prototype.toString.call(location) === '[object Location]' \
         && Object.prototype.toString.call(navigator) === '[object Navigator]' \
         && Object.prototype.toString.call(console) === '[object console]'",
    );
}

#[test]
fn location_and_navigator_have_interface_objects_with_illegal_constructors() {
    let mut rt = rt();
    ok(
        &mut rt,
        "typeof Location === 'function' && typeof Navigator === 'function' \
         && location instanceof Location && navigator instanceof Navigator",
    );
    ok(
        &mut rt,
        "try { new Location(); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { new Navigator(); false } catch (e) { e instanceof TypeError }",
    );
}

// ---- LocationParts (pure Rust, no JS engine needed) ------------------

#[test]
fn location_parts_parses_a_full_url() {
    let parts = super::LocationParts::parse("https://example.test:8443/a/b.html?q=1#h");
    assert_eq!(parts.href, "https://example.test:8443/a/b.html?q=1#h");
    assert_eq!(parts.protocol, "https:");
    assert_eq!(parts.host, "example.test:8443");
    assert_eq!(parts.hostname, "example.test");
    assert_eq!(parts.port, "8443");
    assert_eq!(parts.pathname, "/a/b.html");
    assert_eq!(parts.search, "?q=1");
    assert_eq!(parts.hash, "#h");
    assert_eq!(parts.origin, "https://example.test:8443");
}

#[test]
fn location_parts_parses_a_url_with_no_port_query_fragment_or_path() {
    let parts = super::LocationParts::parse("https://example.test");
    assert_eq!(parts.protocol, "https:");
    assert_eq!(parts.host, "example.test");
    assert_eq!(parts.hostname, "example.test");
    assert_eq!(parts.port, "");
    assert_eq!(parts.pathname, "/");
    assert_eq!(parts.search, "");
    assert_eq!(parts.hash, "");
    assert_eq!(parts.origin, "https://example.test");
}

#[test]
fn location_parts_normalizes_a_bare_query_or_fragment_delimiter() {
    let parts = super::LocationParts::parse("https://example.test/a?");
    assert_eq!(parts.search, "");
    let parts = super::LocationParts::parse("https://example.test/a#");
    assert_eq!(parts.hash, "");
}

#[test]
fn location_parts_degrades_without_a_scheme_separator() {
    let parts = super::LocationParts::parse("not-a-url");
    assert_eq!(parts.href, "not-a-url");
    assert_eq!(parts.protocol, "");
    assert_eq!(parts.host, "");
    assert_eq!(parts.pathname, "not-a-url");
    assert_eq!(parts.origin, "null");
}

#[test]
fn location_parts_about_blank() {
    let parts = super::LocationParts::about_blank();
    assert_eq!(parts.href, "about:blank");
    assert_eq!(parts.protocol, "about:");
    assert_eq!(parts.pathname, "blank");
    assert_eq!(parts.origin, "null");
}

#[test]
fn named_window_access_follows_connected_ids_and_keeps_builtin_members() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "var named = document.createElement('div'); named.id = 'namedTarget'; document.body.appendChild(named); namedTarget === named && window.namedTarget === named",
    );
    ok(
        &mut runtime,
        "named.id = 'renamedTarget'; typeof namedTarget === 'undefined' && renamedTarget === named",
    );
    ok(
        &mut runtime,
        "named.id = 'console'; window.console !== named && typeof console.log === 'function'",
    );
    ok(
        &mut runtime,
        "named.id = 'renamedTarget'; named.remove(); typeof renamedTarget === 'undefined'",
    );
}

#[test]
fn named_window_duplicates_are_live_and_name_attributes_have_html_scope() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "var first = document.createElement('div'); first.id = 'namedGroup'; document.body.appendChild(first); var second = document.createElement('div'); second.id = 'namedGroup'; document.body.appendChild(second); var group = namedGroup; group instanceof HTMLCollection && group.length === 2 && group[0] === first && group[1] === second",
    );
    ok(
        &mut runtime,
        "second.remove(); group.length === 1 && namedGroup === first",
    );
    ok(
        &mut runtime,
        "var form = document.createElement('form'); form.setAttribute('name', 'namedForm'); document.body.appendChild(form); namedForm === form",
    );
    ok(
        &mut runtime,
        "var div = document.createElement('div'); div.setAttribute('name', 'notNamed'); document.body.appendChild(div); typeof notNamed === 'undefined'",
    );
    ok(
        &mut runtime,
        "var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.id = 'notHtml'; document.body.appendChild(svg); typeof notHtml === 'undefined'",
    );
    ok(
        &mut runtime,
        "var prior = Object.getPrototypeOf(window); var shadow = document.createElement('div'); shadow.id = 'namedShadow'; document.body.appendChild(shadow); window.namedShadow = 7; namedShadow === 7",
    );
}

#[test]
fn named_window_property_precedes_object_prototype() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "var element = document.createElement('div'); element.id = 'toString'; document.body.appendChild(element); window.toString === element && toString === element",
    );
    ok(
        &mut runtime,
        "element.remove(); typeof window.toString === 'function'",
    );
}

#[test]
fn named_nodes_rejects_empty_name_without_tree_walk() {
    let document = Document::new();
    assert!(super::named::nodes(&document, "").is_empty());
    let mut runtime = rt();
    ok(&mut runtime, "typeof window[''] === 'undefined'");
}

#[test]
fn named_window_index_key_resolves_numeric_id() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "var zero = document.createElement('div'); zero.id = '0'; document.body.appendChild(zero); window[0] === zero && window['0'] === zero && (0 in window)",
    );
    ok(
        &mut runtime,
        "zero.remove(); typeof window[0] === 'undefined' && !(0 in window)",
    );
}

#[test]
fn named_window_symbol_keys_fall_through_to_target() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "typeof window[Symbol('namedSymbol')] === 'undefined'",
    );
    ok(&mut runtime, "!(Symbol('namedHas') in window)");
}

#[test]
fn named_install_reports_when_global_is_non_extensible() {
    let mut context = boa_engine::Context::default();
    context
        .eval(boa_engine::Source::from_bytes(
            "Object.preventExtensions(globalThis);",
        ))
        .unwrap();
    let err = super::named::install(&mut context)
        .expect_err("frozen global should reject the named-properties prototype");
    assert!(
        format!("{err:?}").contains("cannot install Window named properties"),
        "unexpected install error: {err:?}"
    );
}

#[test]
fn viewport_scroll_starts_at_origin_with_aliases() {
    let mut rt = rt();
    ok(
        &mut rt,
        "scrollX === 0 && scrollY === 0 && pageXOffset === 0 && pageYOffset === 0 \
         && typeof scrollX === 'number' && typeof scrollY === 'number'",
    );
    ok(
        &mut rt,
        "typeof scroll === 'function' && typeof scrollTo === 'function' \
         && typeof scrollBy === 'function' && typeof window.scrollBy === 'function'",
    );
}

#[test]
fn scroll_to_with_numbers_sets_and_clamps() {
    let mut rt = rt();
    rt.evaluate("scrollTo(10, 20);").unwrap();
    ok(
        &mut rt,
        "scrollX === 10 && scrollY === 20 && pageXOffset === 10",
    );
    rt.evaluate("scrollTo(-5, -7);").unwrap();
    ok(&mut rt, "scrollX === 0 && scrollY === 0");
    rt.evaluate("scrollTo(NaN, Infinity);").unwrap();
    ok(&mut rt, "scrollX === 0 && scrollY === 0");
    rt.evaluate("scrollTo();").unwrap();
    ok(&mut rt, "scrollX === 0 && scrollY === 0");
    rt.evaluate("scrollTo(3);").unwrap();
    ok(&mut rt, "scrollX === 3 && scrollY === 0");
}

#[test]
fn scroll_by_accumulates_and_ignores_non_finite_deltas() {
    let mut rt = rt();
    rt.evaluate("scrollBy(5, 7);").unwrap();
    ok(&mut rt, "scrollX === 5 && scrollY === 7");
    rt.evaluate("scrollBy(2, 3);").unwrap();
    ok(&mut rt, "scrollX === 7 && scrollY === 10");
    rt.evaluate("scrollBy(-20, -4);").unwrap();
    ok(&mut rt, "scrollX === 0 && scrollY === 6");
    rt.evaluate("scrollBy(NaN, Infinity);").unwrap();
    ok(&mut rt, "scrollX === 0 && scrollY === 6");
    rt.evaluate("scrollBy();").unwrap();
    ok(&mut rt, "scrollX === 0 && scrollY === 6");
}

#[test]
fn scroll_to_with_options_keeps_missing_axes() {
    let mut rt = rt();
    rt.evaluate("scrollTo(10, 20);").unwrap();
    rt.evaluate("scrollTo({top: 100});").unwrap();
    ok(&mut rt, "scrollX === 10 && scrollY === 100");
    rt.evaluate("scrollTo({left: 30});").unwrap();
    ok(&mut rt, "scrollX === 30 && scrollY === 100");
    rt.evaluate("scrollTo({});").unwrap();
    ok(&mut rt, "scrollX === 30 && scrollY === 100");
    rt.evaluate("scrollTo({left: -5, top: NaN});").unwrap();
    ok(&mut rt, "scrollX === 0 && scrollY === 0");
    rt.evaluate("scrollTo({behavior: 'smooth', left: 4, top: 5});")
        .unwrap();
    ok(&mut rt, "scrollX === 4 && scrollY === 5");
}

#[test]
fn scroll_by_with_options_uses_deltas() {
    let mut rt = rt();
    rt.evaluate("scrollBy({left: 5, top: 7});").unwrap();
    ok(&mut rt, "scrollX === 5 && scrollY === 7");
    rt.evaluate("scrollBy({left: 2});").unwrap();
    ok(&mut rt, "scrollX === 7 && scrollY === 7");
    rt.evaluate("scrollBy({});").unwrap();
    ok(&mut rt, "scrollX === 7 && scrollY === 7");
    rt.evaluate("scrollBy({left: NaN, top: Infinity});")
        .unwrap();
    ok(&mut rt, "scrollX === 7 && scrollY === 7");
}

#[test]
fn scroll_behavior_validates_enum() {
    let mut rt = rt();
    for behavior in ["auto", "instant", "smooth"] {
        let src = format!("scrollTo({{behavior: '{behavior}'}}); true");
        ok(&mut rt, &src);
    }
    let err = rt
        .evaluate("scrollTo({behavior: 'bogus'});")
        .expect_err("unknown behavior should throw");
    assert!(
        format!("{err:?}").contains("behavior is not a valid ScrollBehavior"),
        "unexpected error: {err:?}"
    );
    let err = rt
        .evaluate("scrollBy({behavior: 'bogus'});")
        .expect_err("unknown behavior should throw");
    assert!(
        format!("{err:?}").contains("behavior is not a valid ScrollBehavior"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn scroll_is_an_alias_for_scroll_to() {
    let mut rt = rt();
    rt.evaluate("scroll(11, 22);").unwrap();
    ok(&mut rt, "scrollX === 11 && scrollY === 22");
    rt.evaluate("scroll({left: 1, top: 2});").unwrap();
    ok(&mut rt, "scrollX === 1 && scrollY === 2");
}

#[test]
fn viewport_scroll_attributes_are_read_only() {
    let mut rt = rt();
    rt.evaluate("scrollTo(9, 8);").unwrap();
    // A non-strict write to a getter-only accessor is ignored, not stored.
    rt.evaluate("scrollX = 100; scrollY = 100;").unwrap();
    ok(&mut rt, "scrollX === 9 && scrollY === 8");
}
