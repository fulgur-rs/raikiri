use boa_engine::JsString;
use boa_engine::property::Attribute;

use super::super::interfaces::wrap;
use super::super::test_host::StubHost;
use super::super::webidl::with_state;
use super::super::{DomRuntime, RuntimeError};

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

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

/// Expose the node at `index` to scripts as global `name`.
fn expose(rt: &mut DomRuntime, name: &str, index: usize) {
    let object = wrap(rt.context_mut(), index).unwrap();
    rt.context_mut()
        .register_global_property(JsString::from(name), object, Attribute::all())
        .unwrap();
}

/// One extra node of every kind `Node`/`Element` cover, added under `body`:
/// a text node, a comment, a processing instruction, a template's detached
/// content-fragment root, and a foreign (SVG) element.
struct OtherKinds {
    text: usize,
    comment: usize,
    pi: usize,
    fragment: usize,
    svg: usize,
}

fn other_kinds(rt: &mut DomRuntime, body: usize) -> OtherKinds {
    with_state(rt.context_mut(), |s| {
        let doc = s.host.document_mut();
        let text = doc.append_text(body, "hi");
        let comment = doc.append_comment(Some(body), "note");
        let pi = doc.append_processing_instruction(Some(body), "target", "data");
        let template = doc.create_detached_element("template").unwrap();
        doc.append_child(body, template).unwrap();
        let fragment = doc.allocate_template_fragment_root(template);
        let svg = doc.create_detached_element("svg").unwrap();
        doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
        doc.append_child(body, svg).unwrap();
        OtherKinds {
            text,
            comment,
            pi,
            fragment,
            svg,
        }
    })
    .unwrap()
}

#[test]
fn wrappers_are_identical_and_keep_expandos() {
    let mut rt = rt();
    ok(&mut rt, "document.body === document.body");
    rt.evaluate("document.body.marker = 'm';").unwrap();
    ok(&mut rt, "document.querySelector('body').marker === 'm'");
    ok(
        &mut rt,
        "document.body instanceof HTMLElement && document.body.parentNode === document.documentElement",
    );
    ok(
        &mut rt,
        "document.documentElement.parentNode === document && document.parentNode === null",
    );
    ok(&mut rt, "document.documentElement.parentElement === null");
}

#[test]
fn create_append_and_attributes_round_trip() {
    let mut rt = rt();
    rt.evaluate("var d = document.createElement('DIV'); d.setAttribute('id', 't'); document.body.appendChild(d);").unwrap();
    ok(
        &mut rt,
        "document.getElementById('t') === d && d.tagName === 'DIV' && d.localName === 'div'",
    );
    ok(
        &mut rt,
        "d.nodeName === 'DIV' && d.nodeType === 1 && d.id === 't'",
    );
    ok(
        &mut rt,
        "d.hasAttribute('id') && d.getAttribute('missing') === null",
    );
    rt.evaluate("d.removeAttribute('id');").unwrap();
    ok(&mut rt, "document.getElementById('t') === null");
    ok(&mut rt, "document.getElementById('') === null");
}

#[test]
fn text_content_reads_and_replaces_descendants() {
    let mut rt = rt();
    rt.evaluate(
        "var p = document.createElement('p'); document.body.appendChild(p); p.textContent = 'ab';",
    )
    .unwrap();
    ok(
        &mut rt,
        "p.textContent === 'ab' && document.body.textContent === 'ab'",
    );
    ok(&mut rt, "document.textContent === null");
}

#[test]
fn text_content_covers_text_nodes_null_clear_and_document_no_op() {
    let (mut rt, body) = rt_with_body();
    let k = other_kinds(&mut rt, body);
    expose(&mut rt, "text", k.text);
    // Text.textContent getter (the `NodeKind::Text` arm).
    ok(&mut rt, "text.textContent === 'hi'");
    // Document.textContent has no setter effect (DOM §4.4 step 1: only
    // DocumentFragment / Element / Attr / CharacterData accept a write).
    rt.evaluate("document.textContent = 'ignored';").unwrap();
    ok(&mut rt, "document.textContent === null");
    // A `null` argument clears the element (`LegacyNullToEmptyString`-like).
    rt.evaluate("var p = document.createElement('p'); document.body.appendChild(p); p.textContent = 'ab'; p.textContent = null;").unwrap();
    ok(&mut rt, "p.textContent === ''");
}

#[test]
fn class_list_add_remove_toggle_contains() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; b.classList.add('x', 'y'); b.classList.remove('x');")
        .unwrap();
    ok(
        &mut rt,
        "b.getAttribute('class') === 'y' && b.classList.contains('y') && !b.classList.contains('x')",
    );
    ok(
        &mut rt,
        "b.classList.toggle('z') === true && b.classList.toggle('z') === false",
    );
    ok(&mut rt, "b.classList === b.classList");
    ok(
        &mut rt,
        "try { b.classList.add('a b'); false } catch (e) { e instanceof DOMException && e.name === 'InvalidCharacterError' }",
    );
    ok(
        &mut rt,
        "try { b.classList.add(''); false } catch (e) { e.name === 'SyntaxError' }",
    );
}

#[test]
fn class_list_toggle_force_argument_short_circuits_without_writing() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; b.classList.add('y');")
        .unwrap();
    // `force` already matches the current membership: no attribute write, `want` returned as-is.
    ok(&mut rt, "b.classList.toggle('y', true) === true");
    ok(&mut rt, "b.classList.toggle('q', false) === false");
    ok(&mut rt, "b.getAttribute('class') === 'y'");
}

#[test]
fn class_list_function_lengths_reflect_webidl_arity() {
    let mut rt = rt();
    // `add`/`remove` take a variadic token list (WebIDL length excludes it);
    // `contains`/`toggle` have one mandatory argument.
    ok(&mut rt, "document.body.classList.add.length === 0");
    ok(&mut rt, "document.body.classList.remove.length === 0");
    ok(&mut rt, "document.body.classList.contains.length === 1");
    ok(&mut rt, "document.body.classList.toggle.length === 1");
}

#[test]
fn query_selector_uses_full_selectors_in_tree_order() {
    let mut rt = rt();
    rt.evaluate(
        "var a = document.createElement('div'); a.setAttribute('class', 'c');
         var b = document.createElement('span'); a.appendChild(b);
         document.body.appendChild(a);",
    )
    .unwrap();
    ok(&mut rt, "document.querySelector('div.c > span') === b");
    ok(&mut rt, "document.querySelector('.c') === a");
    ok(&mut rt, "document.querySelector('p') === null");
    ok(
        &mut rt,
        "try { document.querySelector('p['); false } catch (e) { e instanceof DOMException && e.name === 'SyntaxError' && e.code === 12 }",
    );
}

#[test]
fn query_selector_returns_the_first_match_in_document_order() {
    let mut rt = rt();
    rt.evaluate(
        "var first = document.createElement('p'); document.body.appendChild(first);
         var second = document.createElement('p'); document.body.appendChild(second);",
    )
    .unwrap();
    ok(&mut rt, "document.querySelector('p') === first");
}

#[test]
fn query_selector_root_matches_document_element() {
    let mut rt = rt();
    ok(
        &mut rt,
        "document.querySelector(':root') === document.documentElement",
    );
}

/// Models what a real embedder does: its layout pass refreshes
/// `IS_IN_DOCUMENT` on entry (independently of any `querySelector` call),
/// and script then mutates the tree afterward. Here that refresh is done
/// directly against the document while `i` is still detached, so its bit
/// is correctly cleared; script then attaches `i` as `b`'s previous
/// sibling, and nothing else recomputes the bit on its own. A sibling
/// combinator's candidate lookup consults the bit directly (unlike
/// child/descendant combinators, which walk this binding's own
/// always-accurate ancestor list), so without a fresh refresh right before
/// `querySelector`'s own walk, `i`'s bit would still say "not in document"
/// and `i + b` would wrongly fail to match `b`, a plain later sibling.
#[test]
fn query_selector_uses_fresh_in_document_flags() {
    let mut rt = rt();
    rt.evaluate("var i = document.createElement('i');").unwrap();
    with_state(rt.context_mut(), |s| {
        s.host.document_mut().mark_in_document_flags();
    })
    .unwrap();
    rt.evaluate(
        "document.body.appendChild(i);
         var b = document.createElement('b');
         document.body.appendChild(b);",
    )
    .unwrap();
    ok(&mut rt, "document.querySelector('i + b') === b");
}

#[test]
fn hierarchy_and_name_errors_are_dom_exceptions() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { document.createElement('1bad'); false } catch (e) { e.name === 'InvalidCharacterError' }",
    );
    ok(
        &mut rt,
        "try { document.body.appendChild(document.documentElement); false } catch (e) { e.name === 'HierarchyRequestError' }",
    );
    ok(
        &mut rt,
        "try { document.body.appendChild({}); false } catch (e) { e instanceof TypeError }",
    );
}

#[test]
fn append_child_rejects_non_element_parents_and_document_children() {
    let (mut rt, body) = rt_with_body();
    let k = other_kinds(&mut rt, body);
    expose(&mut rt, "text", k.text);
    // `parent_ok` is false: a Text node cannot receive children.
    ok(
        &mut rt,
        "try { text.appendChild(document.createElement('i')); false } catch (e) { e.name === 'HierarchyRequestError' }",
    );
    // `child_is_document` is true: a Document can never be inserted as a child.
    ok(
        &mut rt,
        "try { document.body.appendChild(document); false } catch (e) { e.name === 'HierarchyRequestError' }",
    );
    ok(
        &mut rt,
        "var e = document.createElement('em'); document.body.appendChild(e) === e",
    );
}

#[test]
fn attribute_names_are_validated_as_dom_exceptions() {
    let mut rt = rt();
    rt.evaluate("var d = document.createElement('div');")
        .unwrap();
    ok(
        &mut rt,
        "try { d.setAttribute('1bad', 'x'); false } catch (e) { e instanceof DOMException && e.name === 'InvalidCharacterError' }",
    );
    // DOM §4.9: `removeAttribute` never validates its argument -- it looks
    // an attribute up by name and removes it if found, so a syntactically
    // invalid name is simply never found. Unlike `setAttribute`, this is a
    // silent no-op, not a `DOMException`.
    ok(&mut rt, "d.removeAttribute('1bad') === undefined");
}

#[test]
fn element_member_on_non_element_is_type_error_not_panic() {
    let mut rt = rt();
    let err = rt.evaluate("Element.prototype.getAttribute.call(document, 'x')");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "{err:?}"
    );
    ok(&mut rt, "document.body !== null");
}

#[test]
fn document_head_and_body_parent_element() {
    let mut rt = rt();
    ok(&mut rt, "document.head.localName === 'head'");
    ok(
        &mut rt,
        "document.body.parentElement === document.documentElement",
    );
}

#[test]
fn node_name_covers_every_kind() {
    let (mut rt, body) = rt_with_body();
    let k = other_kinds(&mut rt, body);
    expose(&mut rt, "text", k.text);
    expose(&mut rt, "comment", k.comment);
    expose(&mut rt, "pi", k.pi);
    expose(&mut rt, "fragment", k.fragment);
    expose(&mut rt, "svg", k.svg);
    ok(&mut rt, "document.nodeName === '#document'");
    ok(&mut rt, "text.nodeName === '#text'");
    ok(&mut rt, "comment.nodeName === '#comment'");
    // No dedicated `ProcessingInstruction` interface (and its `target`
    // member) exists yet; the generic `Node.nodeName` fallback is `''`.
    ok(&mut rt, "pi.nodeName === ''");
    ok(&mut rt, "fragment.nodeName === '#document-fragment'");
    // A non-HTML-namespace element's tag name is not uppercased.
    ok(&mut rt, "svg.nodeName === 'svg'");
}

#[test]
fn mutations_mark_the_host_dirty_once_per_read() {
    let (host, ..) = StubHost::page();
    let flushes = host.flushes.clone();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("document.body.setAttribute('class', 'q');")
        .unwrap();
    assert_eq!(flushes.get(), 0, "mutations alone never flush");
}

/// `find_in_tree`'s walk must not use the native call stack: a script can
/// build an arbitrarily deep chain, and there is no other bound on it
/// before it reaches raikiri-dom. Built directly against `Document` with
/// `create_detached_element` + `attach_child` (both O(1)) rather than
/// through JS `appendChild`, whose cycle check does an O(N) `parent_of`
/// scan per call and would make building this chain quadratic.
#[test]
fn find_in_tree_walks_a_very_deep_chain_without_overflowing_the_stack() {
    let (mut rt, body) = rt_with_body();
    let deep = with_state(rt.context_mut(), |s| {
        let doc = s.host.document_mut();
        let mut parent = body;
        for _ in 0..100_000 {
            let child = doc.create_detached_element("div").unwrap();
            doc.attach_child(parent, child);
            parent = child;
        }
        doc.set_element_attribute(parent, "id", "deep").unwrap();
        parent
    })
    .unwrap();
    expose(&mut rt, "deep", deep);
    ok(&mut rt, "document.getElementById('deep') === deep");
    ok(&mut rt, "document.querySelector('#deep') === deep");
}

fn is_dirty(rt: &mut DomRuntime) -> bool {
    with_state(rt.context_mut(), |s| s.dirty).unwrap()
}

fn clear_dirty(rt: &mut DomRuntime) {
    with_state(rt.context_mut(), |s| s.dirty = false).unwrap();
}

#[test]
fn inner_html_serializes_and_replaces_children_through_the_host_parser() {
    let mut rt = rt();
    rt.evaluate(
        "var p = document.createElement('p'); document.body.appendChild(p); p.textContent = 'a<b';",
    )
    .unwrap();
    ok(&mut rt, "p.innerHTML === 'a&lt;b'");
    // `StubHost::parse_fragment` turns markup into a single text child.
    rt.evaluate("p.innerHTML = 'xyz';").unwrap();
    ok(&mut rt, "p.textContent === 'xyz'");
}

/// `document.createElement('template')` must allocate a template-contents
/// fragment root the same way the HTML parser does, so `innerHTML` writes
/// end up in that fragment rather than as the template element's own
/// children: a script-created `<template>` has no visible `textContent`.
#[test]
fn create_element_wires_a_template_contents_fragment_for_script_created_templates() {
    let mut rt = rt();
    rt.evaluate(
        "var t = document.createElement('template'); document.body.appendChild(t); t.innerHTML = 'x';",
    )
    .unwrap();
    // The markup did land somewhere (in the template's contents fragment,
    // per `Document::serialize_inner_html`'s own template handling) --
    // `textContent` being empty is not just `innerHTML` silently dropping it.
    ok(&mut rt, "t.innerHTML === 'x' && t.textContent === ''");
}

/// `[LegacyNullToEmptyString]` (the `innerHTML` attribute's WebIDL type):
/// `null` sets the empty string, not the string `"null"` that plain
/// `ToString` conversion would otherwise produce.
#[test]
fn inner_html_setter_treats_null_as_the_empty_string() {
    let mut rt = rt();
    rt.evaluate(
        "var p = document.createElement('p'); document.body.appendChild(p); p.innerHTML = null;",
    )
    .unwrap();
    ok(&mut rt, "p.textContent === ''");
}

/// `serialize_inner_html` rejects an attribute name that is not a valid XML
/// `Name`, a check specific to `Element.setAttribute` (DOM §4.9), not to
/// HTML fragment serialization. A real HTML parser can legitimately produce
/// such a name (e.g. one starting with an ASCII digit); `1bad` is planted
/// directly (bypassing `setAttribute`'s own XML `Name` validation, since
/// `Document::set_element_attributes` -- the plural form
/// `replace_children_from` uses -- performs none) to model that, without
/// depending on a fragment-parsing host implementation.
#[test]
fn inner_html_getter_reports_an_invalid_attribute_name_as_a_host_error() {
    let (mut rt, body) = rt_with_body();
    with_state(rt.context_mut(), |s| {
        let doc = s.host.document_mut();
        let child = doc.create_detached_element("div").unwrap();
        doc.append_child(body, child).unwrap();
        doc.set_element_attributes(child, vec![("1bad".into(), "x".into())]);
    })
    .unwrap();
    let err = rt.evaluate("document.body.innerHTML;");
    assert!(
        matches!(err, Err(RuntimeError::Host(ref m)) if m.contains("invalid attribute name")),
        "{err:?}"
    );
}

/// The arena index in raikiri-dom's own error message (`... on innerHTML
/// node {id}`) must never reach script: a script that catches the getter's
/// exception should see a fixed message with no digits from that index,
/// even though the harness-facing host failure above still records the
/// original, more detailed message.
#[test]
fn inner_html_getter_js_visible_message_hides_the_node_index() {
    let (mut rt, body) = rt_with_body();
    with_state(rt.context_mut(), |s| {
        let doc = s.host.document_mut();
        let child = doc.create_detached_element("div").unwrap();
        doc.append_child(body, child).unwrap();
        doc.set_element_attributes(child, vec![("1bad".into(), "x".into())]);
    })
    .unwrap();
    // The overall `evaluate` call still reports a host failure (the recorded
    // detail, not the caught exception's message), so the JS-visible message
    // is stashed into a global for a second, unrelated `evaluate` call to
    // read back.
    let _ = rt.evaluate(
        "var caughtMessage = ''; \
         try { document.body.innerHTML; } catch (e) { caughtMessage = e.message; }",
    );
    let message = rt
        .evaluate("caughtMessage")
        .unwrap()
        .to_string(rt.context_mut())
        .unwrap()
        .to_std_string_escaped();
    assert!(
        !message.chars().any(|c| c.is_ascii_digit()),
        "node index leaked into the JS-visible message: {message:?}"
    );
    assert!(message.contains("innerHTML"), "{message:?}");
}

#[test]
fn inner_html_host_failure_is_reported_as_host_error() {
    use super::super::host::{BoxGeometry, DocumentHost, HostError};

    /// A host whose fragment parser always fails, to exercise the
    /// `innerHTML` setter's host-failure path.
    struct FailingParse(StubHost);
    impl DocumentHost for FailingParse {
        fn document(&self) -> &raikiri_dom::Document {
            self.0.document()
        }
        fn document_mut(&mut self) -> &mut raikiri_dom::Document {
            self.0.document_mut()
        }
        fn flush(&mut self) -> Result<(), HostError> {
            self.0.flush()
        }
        fn box_geometry(&mut self, node: usize) -> Result<Option<BoxGeometry>, HostError> {
            self.0.box_geometry(node)
        }
        fn computed_value(
            &mut self,
            node: usize,
            property: &str,
        ) -> Result<Option<String>, HostError> {
            self.0.computed_value(node, property)
        }
        fn parse_fragment(
            &mut self,
            _context_tag: &str,
            _context_ns: &str,
            _markup: &str,
        ) -> Result<raikiri_dom::Document, HostError> {
            Err(HostError("parser unavailable".into()))
        }
    }

    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(FailingParse(host)).unwrap();
    let err = rt.evaluate("document.body.innerHTML = '<p></p>';");
    assert_eq!(err, Err(RuntimeError::Host("parser unavailable".into())));
}

#[test]
fn mutations_set_dirty_and_reads_and_no_op_writes_do_not() {
    let mut rt = rt();
    rt.evaluate(
        "var d = document.createElement('div'); document.body.appendChild(d);
         d.setAttribute('id', 't');",
    )
    .unwrap();

    clear_dirty(&mut rt);
    rt.evaluate("d.setAttribute('class', 'q');").unwrap();
    assert!(is_dirty(&mut rt), "setAttribute should mark dirty");

    clear_dirty(&mut rt);
    rt.evaluate("document.body.appendChild(document.createElement('span'));")
        .unwrap();
    assert!(is_dirty(&mut rt), "appendChild should mark dirty");

    clear_dirty(&mut rt);
    rt.evaluate("d.textContent = 'x';").unwrap();
    assert!(is_dirty(&mut rt), "textContent= should mark dirty");

    clear_dirty(&mut rt);
    rt.evaluate("d.classList.add('y');").unwrap();
    assert!(is_dirty(&mut rt), "classList.add should mark dirty");

    clear_dirty(&mut rt);
    rt.evaluate("d.getAttribute('id'); document.querySelector('div');")
        .unwrap();
    assert!(!is_dirty(&mut rt), "reads should not mark dirty");

    clear_dirty(&mut rt);
    rt.evaluate("d.removeAttribute('1bad');").unwrap();
    assert!(
        !is_dirty(&mut rt),
        "an invalid removeAttribute name is a no-op, not a mutation"
    );

    clear_dirty(&mut rt);
    // `d`'s class is `y` (set above); `force` already matches, so this is a no-write no-op.
    rt.evaluate("d.classList.toggle('y', true);").unwrap();
    assert!(
        !is_dirty(&mut rt),
        "a toggle matching the current state should not write"
    );

    clear_dirty(&mut rt);
    rt.evaluate("d.innerHTML = '<span></span>';").unwrap();
    assert!(is_dirty(&mut rt), "innerHTML= should mark dirty");

    clear_dirty(&mut rt);
    rt.evaluate("d.innerHTML;").unwrap();
    assert!(
        !is_dirty(&mut rt),
        "reading innerHTML should not mark dirty"
    );
}
