use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;

fn rt() -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

/// [`rt`] plus the fixture page's `<body>` arena index, for tests that build
/// non-element nodes directly against the document.
fn rt_with_body() -> (DomRuntime, usize) {
    let (host, _, _, body) = StubHost::page();
    (DomRuntime::new(host).unwrap(), body)
}

/// A runtime over a hand-built `document`, for tests that need a document
/// shape [`StubHost::page`]'s fixed `<html><head></head><body></body></html>`
/// cannot produce (a non-HTML document element, or an `<html>` with no
/// `<head>` at all).
fn rt_over(document: raikiri_dom::Document) -> DomRuntime {
    let host = StubHost {
        document,
        flushes: std::rc::Rc::new(std::cell::Cell::new(0)),
        geometry: std::collections::HashMap::new(),
        computed: std::collections::HashMap::new(),
        fail_flush: false,
        fail_geometry: false,
        fail_computed: false,
    };
    DomRuntime::new(host).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

#[test]
fn create_element_ns_builds_a_foreign_element_and_reports_its_namespace() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var s = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); \
         s.namespaceURI === 'http://www.w3.org/2000/svg' && s.localName === 'svg' \
         && s.tagName === 'svg' && !(s instanceof HTMLElement) && s instanceof Element",
    );
}

#[test]
fn create_element_ns_with_the_html_namespace_matches_create_element() {
    let mut rt = rt();
    ok(
        &mut rt,
        "document.createElementNS('http://www.w3.org/1999/xhtml', 'div') instanceof HTMLElement",
    );
    // The HTML-namespace path allocates a template-contents fragment root,
    // the same as `createElement('template')`.
    ok(
        &mut rt,
        "var t = document.createElementNS('http://www.w3.org/1999/xhtml', 'template'); \
         document.body.appendChild(t); t.innerHTML = 'x'; t.textContent === ''",
    );
}

#[test]
fn create_element_ns_rejects_an_invalid_qualified_name() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { document.createElementNS(null, '1x'); false } \
         catch (e) { e.name === 'InvalidCharacterError' }",
    );
    ok(
        &mut rt,
        "try { document.createElementNS('urn:x', 'a:b:c'); false } \
         catch (e) { e.name === 'InvalidCharacterError' }",
    );
}

#[test]
fn create_element_ns_null_namespace_element_is_not_html() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var n = document.createElementNS(null, 'thing'); \
         n.namespaceURI === null && !(n instanceof HTMLElement) && n instanceof Element",
    );
    // An empty-string namespace normalizes to `null`, per "validate and
    // extract"'s own first step.
    ok(
        &mut rt,
        "document.createElementNS('', 'thing').namespaceURI === null",
    );
}

#[test]
fn create_element_ns_enforces_prefix_namespace_combinations() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { document.createElementNS(null, 'a:b'); false } \
         catch (e) { e.name === 'NamespaceError' }",
    );
    ok(
        &mut rt,
        "try { document.createElementNS('urn:x', 'xml:b'); false } \
         catch (e) { e.name === 'NamespaceError' }",
    );
    ok(
        &mut rt,
        "try { document.createElementNS('urn:x', 'xmlns:b'); false } \
         catch (e) { e.name === 'NamespaceError' }",
    );
    ok(
        &mut rt,
        "try { document.createElementNS('http://www.w3.org/2000/xmlns/', 'b'); false } \
         catch (e) { e.name === 'NamespaceError' }",
    );
    ok(
        &mut rt,
        "document.createElementNS('http://www.w3.org/XML/1998/namespace', 'xml:b') instanceof Element",
    );
}

#[test]
fn document_title_getter_normalizes_and_setter_creates_or_replaces() {
    let mut rt = rt();
    ok(&mut rt, "document.title === ''");
    rt.evaluate("document.title = '  a   b ';").unwrap();
    ok(
        &mut rt,
        "document.title === 'a b' && document.head.querySelector('title').textContent === '  a   b '",
    );
    // A second write replaces the same title element's text rather than
    // creating another one.
    rt.evaluate("document.title = 'c';").unwrap();
    ok(
        &mut rt,
        "document.title === 'c' \
         && document.head.querySelectorAll('title').length === 1",
    );
}

#[test]
fn document_title_getter_ignores_a_non_html_title_element() {
    let (mut rt, body) = rt_with_body();
    with_state(rt.context_mut(), |s| {
        let doc = s.host.document_mut();
        let svg_title = doc.create_detached_element("title").unwrap();
        doc.set_element_namespace(svg_title, Some("http://www.w3.org/2000/svg".into()));
        doc.append_child(body, svg_title).unwrap();
        doc.append_text(svg_title, "not html");
    })
    .unwrap();
    ok(&mut rt, "document.title === ''");
}

/// The setter's "document element exists but is not HTML" no-op branch: a
/// document whose only child is a foreign-namespace root (standing in for
/// an SVG document, whose own title handling this runtime does not
/// implement) never creates or overwrites anything.
#[test]
fn document_title_setter_no_ops_when_the_document_element_is_not_html() {
    let mut document = raikiri_dom::Document::new();
    let root = document.root_index();
    let svg = document.create_detached_element("svg").unwrap();
    document.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    document.attach_child(root, svg);
    document.mark_in_document_flags();
    let mut rt = rt_over(document);
    rt.evaluate("document.title = 'x';").unwrap();
    ok(&mut rt, "document.title === ''");
}

/// The document-element namespace check runs before the `find_html_title`
/// search: a non-HTML document element is a no-op even when an
/// HTML-namespace `title` element happens to exist somewhere else in such a
/// document (a shape `find_html_title`'s own whole-document walk would
/// otherwise happily find and overwrite).
#[test]
fn document_title_setter_ignores_a_stray_html_title_under_a_non_html_root() {
    let mut document = raikiri_dom::Document::new();
    let root = document.root_index();
    let svg = document.create_detached_element("svg").unwrap();
    document.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    document.attach_child(root, svg);
    let stray_title = document.create_detached_element("title").unwrap();
    document.append_child(svg, stray_title).unwrap();
    document.append_text(stray_title, "stray");
    document.mark_in_document_flags();
    let mut rt = rt_over(document);
    rt.evaluate("document.title = 'x';").unwrap();
    // Unchanged: the getter also finds this title (it only checks
    // namespace, not the document element), but the setter never touched it.
    ok(&mut rt, "document.title === 'stray'");
}

/// `<html></html>` with no `<head>` at all and no title anywhere: there is
/// nowhere to create one, so the setter is a no-op.
#[test]
fn document_title_setter_no_ops_for_html_without_head_or_title() {
    let mut document = raikiri_dom::Document::new();
    let root = document.root_index();
    let html = document.create_detached_element("html").unwrap();
    document.attach_child(root, html);
    document.mark_in_document_flags();
    let mut rt = rt_over(document);
    rt.evaluate("document.title = 'x';").unwrap();
    ok(&mut rt, "document.title === ''");
}

/// `<html><title>old</title></html>`, still with no `<head>`: the setter
/// must still find and overwrite this title through `find_html_title`'s
/// whole-document search, even though there is no `<head>` to fall back to
/// creating one in.
#[test]
fn document_title_setter_overwrites_a_title_outside_head_when_there_is_no_head() {
    let mut document = raikiri_dom::Document::new();
    let root = document.root_index();
    let html = document.create_detached_element("html").unwrap();
    document.attach_child(root, html);
    let title = document.create_detached_element("title").unwrap();
    document.append_child(html, title).unwrap();
    document.append_text(title, "old");
    document.mark_in_document_flags();
    let mut rt = rt_over(document);
    ok(&mut rt, "document.title === 'old'");
    rt.evaluate("document.title = 'new';").unwrap();
    ok(&mut rt, "document.title === 'new'");
}

/// [`super::is_xml_name_start`]/[`super::is_xml_name_char`] duplicate
/// `raikiri_dom`'s own XML `Name` character classes range for range; this
/// mirrors that crate's own direct-function coverage of the same ranges
/// (`raikiri-dom`'s `xml_name_validation_accepts_the_non_ascii_name_ranges`)
/// rather than routing every representative code point through a
/// `createElementNS` call.
#[test]
fn xml_name_character_classes_accept_every_non_ascii_range() {
    for ch in [
        'À', 'Ø', 'ø', 'Ͱ', 'Ϳ', '\u{200c}', '⁰', 'Ⰰ', '々', '豈', 'ﷰ', '𐀀',
    ] {
        assert!(super::is_xml_name_start(ch), "{ch:?}");
    }
    for ch in ['0', '-', '.', '·', '\u{0300}', '\u{203f}'] {
        assert!(super::is_xml_name_char(ch), "{ch:?}");
    }
    assert!(!super::is_xml_name_char('#'));
    assert!(super::is_xml_name("π\u{0300}"));
    assert!(!super::is_xml_name("0name"));
    assert!(!super::is_xml_name("ab#"));
}
