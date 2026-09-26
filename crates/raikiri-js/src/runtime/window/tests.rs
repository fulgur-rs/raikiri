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
