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
    // A `null` argument clears the element ([LegacyNullToEmptyString]-like).
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
fn query_selector_root_matches_document_element() {
    let mut rt = rt();
    ok(
        &mut rt,
        "document.querySelector(':root') === document.documentElement",
    );
}

/// A node created while detached defaults its `IS_IN_DOCUMENT` bit to
/// `true` (raikiri-dom's optimistic default); a node placed inside a
/// `<template>` needs a fresh `mark_in_document_flags` walk to have that bit
/// correctly cleared, because `<template>` content is inert (excluded from
/// the flat tree) but nothing else in the mutation path recomputes the
/// bit eagerly. A sibling combinator's candidate lookup consults the bit
/// directly (unlike child/descendant combinators, which walk this binding's
/// own always-accurate ancestor list), so it is the only kind of selector
/// where stale bits are observable here.
#[test]
fn query_selector_uses_fresh_in_document_flags() {
    let mut rt = rt();
    rt.evaluate(
        "var tmpl = document.createElement('template'); document.body.appendChild(tmpl);
         var i = document.createElement('i'); tmpl.appendChild(i);
         var b = document.createElement('b'); tmpl.appendChild(b);",
    )
    .unwrap();
    ok(&mut rt, "document.querySelector('i + b') === null");
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
    ok(
        &mut rt,
        "try { d.removeAttribute('1bad'); false } catch (e) { e.name === 'InvalidCharacterError' }",
    );
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
