use boa_engine::JsString;
use boa_engine::property::Attribute;

use crate::runtime::interfaces::wrap;
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;
use crate::runtime::{DomRuntime, RuntimeError};

fn rt() -> DomRuntime {
    let (h, ..) = StubHost::page();
    DomRuntime::new(h).unwrap()
}
fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

/// Expose the node at `index` to scripts as global `name`, for a node kind
/// (here, `ProcessingInstruction`) with no script-visible constructor.
fn expose(rt: &mut DomRuntime, name: &str, index: usize) {
    let object = wrap(rt.context_mut(), index).unwrap();
    rt.context_mut()
        .register_global_property(JsString::from(name), object, Attribute::all())
        .unwrap();
}

#[test]
fn sibling_and_child_navigation() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; var x = document.createElement('x'); var t = document.createTextNode('t'); var y = document.createElement('y'); b.append(x, t, y);").unwrap();
    ok(
        &mut rt,
        "b.firstChild === x && b.lastChild === y && x.nextSibling === t && y.previousSibling === t",
    );
    ok(
        &mut rt,
        "x.nextElementSibling === y && y.previousElementSibling === x && b.firstElementChild === x && b.lastElementChild === y && b.childElementCount === 2",
    );
    ok(
        &mut rt,
        "b.childNodes.length === 3 && b.childNodes[1] === t && b.children.length === 2 && b.hasChildNodes()",
    );
    ok(
        &mut rt,
        "x.previousSibling === null && y.nextSibling === null && t.nodeName === '#text' && t.nodeType === 3",
    );
}

#[test]
fn insert_remove_replace_and_errors() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; var a = document.createElement('a'); var c = document.createElement('c'); b.appendChild(c); b.insertBefore(a, c);").unwrap();
    ok(&mut rt, "b.firstChild === a && a.nextSibling === c");
    ok(
        &mut rt,
        "b.insertBefore(document.createElement('z'), null) === b.lastChild",
    );
    ok(
        &mut rt,
        "var r = document.createElement('r'); b.replaceChild(r, a) === a && b.firstChild === r && a.parentNode === null",
    );
    ok(&mut rt, "b.removeChild(c) === c && c.parentNode === null");
    ok(
        &mut rt,
        "try { b.removeChild(c); false } catch (e) { e.name === 'NotFoundError' && e.code === 8 }",
    );
    ok(
        &mut rt,
        "try { b.insertBefore(document.createElement('q'), c); false } catch (e) { e.name === 'NotFoundError' }",
    );
    ok(
        &mut rt,
        "try { document.appendChild(document.createElement('p')); false } catch (e) { e.name === 'HierarchyRequestError' }",
    );
    ok(
        &mut rt,
        "try { document.appendChild(document.createTextNode('x')); false } catch (e) { e.name === 'HierarchyRequestError' }",
    );
    ok(
        &mut rt,
        "try { b.appendChild(document); false } catch (e) { e.name === 'HierarchyRequestError' }",
    );
}

#[test]
fn fragments_move_their_children() {
    let mut rt = rt();
    rt.evaluate("var f = document.createDocumentFragment(); var p = document.createElement('p'); f.append(p, 'txt'); var b = document.body; var m = document.createElement('m'); b.appendChild(m); b.insertBefore(f, m);").unwrap();
    ok(
        &mut rt,
        "f.childNodes.length === 0 && b.firstChild === p && p.nextSibling.data === 'txt' && p.nextSibling.nextSibling === m",
    );
    ok(
        &mut rt,
        "f.nodeType === 11 && f instanceof DocumentFragment && f.parentNode === null",
    );
}

#[test]
fn character_data_members() {
    let mut rt = rt();
    rt.evaluate("var t = document.createTextNode('ab'); var c = document.createComment('cm');")
        .unwrap();
    ok(
        &mut rt,
        "t instanceof Text && t instanceof CharacterData && t.data === 'ab' && t.length === 2 && t.nodeValue === 'ab'",
    );
    rt.evaluate("t.data = 'xyz'; t.appendData('!'); c.nodeValue = 'n2';")
        .unwrap();
    ok(
        &mut rt,
        "t.data === 'xyz!' && t.textContent === 'xyz!' && c.data === 'n2' && c.nodeName === '#comment'",
    );
    rt.evaluate("t.data = null;").unwrap();
    ok(&mut rt, "t.data === ''");
    ok(
        &mut rt,
        "document.body.nodeValue === null && document.nodeValue === null",
    );
    ok(
        &mut rt,
        "try { CharacterData.prototype.appendData.call(document.body, 'x'); false } catch (e) { e instanceof TypeError }",
    );
}

#[test]
fn child_node_mixin_and_text_content_setters() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; var m = document.createElement('m'); b.appendChild(m); m.before('pre', document.createElement('i')); m.after('post'); m.replaceWith(document.createElement('n'));").unwrap();
    ok(
        &mut rt,
        "b.childNodes.length === 4 && b.firstChild.data === 'pre' && b.childNodes[2].localName === 'n' && b.lastChild.data === 'post' && m.parentNode === null",
    );
    rt.evaluate("b.lastChild.remove(); b.prepend('first'); b.replaceChildren('only');")
        .unwrap();
    ok(
        &mut rt,
        "b.childNodes.length === 1 && b.firstChild.data === 'only'",
    );
    rt.evaluate("var f = document.createDocumentFragment(); f.textContent = 'ft'; var t = document.createTextNode('a'); t.textContent = 'b';").unwrap();
    ok(
        &mut rt,
        "f.firstChild.data === 'ft' && t.data === 'b' && f.textContent === 'ft'",
    );
}

#[test]
fn owner_document_is_connected_and_contains() {
    let mut rt = rt();
    rt.evaluate("var d = document.createElement('d');").unwrap();
    ok(
        &mut rt,
        "d.ownerDocument === document && document.ownerDocument === null && !d.isConnected && document.isConnected",
    );
    rt.evaluate("document.body.appendChild(d);").unwrap();
    ok(
        &mut rt,
        "d.isConnected && document.contains(d) && document.body.contains(d) && d.contains(d) && !d.contains(document.body) && !d.contains(null)",
    );
}

#[test]
fn mutations_mark_dirty() {
    let (h, ..) = StubHost::page();
    let mut rt = DomRuntime::new(h).unwrap();
    for src in [
        "document.body.append('x')",
        "document.body.firstChild.data = 'y'",
        "document.body.insertBefore(document.createElement('a'), document.body.firstChild)",
        "document.body.removeChild(document.body.firstChild)",
    ] {
        crate::runtime::webidl::with_state(rt.context_mut(), |s| s.dirty = false).unwrap();
        rt.evaluate(src).unwrap();
        assert!(
            crate::runtime::webidl::with_state(rt.context_mut(), |s| s.dirty).unwrap(),
            "{src}"
        );
    }
    let _ = RuntimeError::Host(String::new());
}

/// `this_parent_node` / `this_child_node` / `this_processing_instruction`
/// each reject an interface they were not installed on, the same as the
/// existing `this_element` / `this_document` brand checks.
#[test]
fn mixin_brand_checks_reject_the_wrong_interface() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { Element.prototype.append.call(document.createTextNode('x')); false } \
         catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { CharacterData.prototype.before.call(document); false } \
         catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { Object.getOwnPropertyDescriptor(ProcessingInstruction.prototype, 'target')\
         .get.call(document.createComment('c')); false } catch (e) { e instanceof TypeError }",
    );
}

/// `ProcessingInstruction.target`, otherwise unreachable from script (there
/// is no `createProcessingInstruction` binding), exercised directly against
/// a raikiri-dom processing instruction exposed as a global.
#[test]
fn processing_instruction_target_getter() {
    let mut rt = rt();
    let pi = with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .append_processing_instruction(None, "xml-stylesheet", "href=\"x\"")
    })
    .unwrap();
    expose(&mut rt, "pi", pi);
    ok(
        &mut rt,
        "pi instanceof ProcessingInstruction && pi.target === 'xml-stylesheet'",
    );
}

/// `arg_node_or_null` (`Node?` arguments, e.g. `Node.contains`'s `other`):
/// a missing argument and an explicit `null`/`undefined` all convert to
/// `None`; anything else that isn't a `Node` wrapper is a `TypeError`.
#[test]
fn nullable_node_argument_conversion() {
    let mut rt = rt();
    ok(&mut rt, "document.body.contains() === false");
    ok(&mut rt, "document.body.contains(undefined) === false");
    ok(&mut rt, "document.body.contains(null) === false");
    ok(
        &mut rt,
        "try { document.body.contains({}); false } catch (e) { e instanceof TypeError }",
    );
}

/// `ChildNode.before` / `after` / `replaceWith` / `remove` are all no-ops on
/// a detached node (DOM §4.2.6: "if parent is null, then return").
#[test]
fn child_node_mixin_no_ops_when_detached() {
    let mut rt = rt();
    rt.evaluate("var d = document.createElement('d');").unwrap();
    ok(
        &mut rt,
        "d.before('x') === undefined && d.after('x') === undefined \
         && d.replaceWith('x') === undefined && d.remove() === undefined",
    );
    ok(&mut rt, "d.parentNode === null");
}

/// `replaceWith`'s replacement argument can itself include `this` (moved,
/// along with the other replacement nodes, into the temporary fragment
/// `nodes_into_a_node` builds) -- `this` is then no longer a child of its
/// old parent by the time `replaceWith` decides how to place the result, so
/// it falls back to inserting before the previously-computed viable next
/// sibling instead of a plain `replace_child`.
#[test]
fn replace_with_handles_this_among_the_replacement_nodes() {
    let mut rt = rt();
    rt.evaluate(
        "var b = document.body; var m = document.createElement('m'); b.appendChild(m); \
         var n = document.createElement('n'); b.appendChild(n); \
         m.replaceWith(n, m);",
    )
    .unwrap();
    ok(
        &mut rt,
        "b.childNodes.length === 2 && b.firstChild === n && b.lastChild === m && m.parentNode === b",
    );
}

/// `m.before(x)` where `x` is already `m`'s immediately preceding sibling:
/// the viable-previous-sibling search must skip `x` itself (it is one of
/// the given nodes), leaving `m`'s reference position unaffected -- a
/// same-place reorder, not a duplicate insertion.
#[test]
fn before_skips_a_sibling_that_is_also_being_inserted() {
    let mut rt = rt();
    rt.evaluate(
        "var b = document.body; var x = document.createElement('x'); b.appendChild(x); \
         var m = document.createElement('m'); b.appendChild(m); \
         m.before(x);",
    )
    .unwrap();
    ok(
        &mut rt,
        "b.childNodes.length === 2 && b.firstChild === x && b.lastChild === m",
    );
}

/// `m.before('x')` where `m` has a real preceding sibling `a` that is not
/// among the given nodes: the viable-previous-sibling search finds `a`
/// itself (rather than falling back to `parent`'s first child), so the new
/// content lands immediately after `a`, not at the very front of `b`.
#[test]
fn before_resolves_a_real_viable_previous_sibling() {
    let mut rt = rt();
    rt.evaluate(
        "var b = document.body; var a = document.createElement('a'); b.appendChild(a); \
         var m = document.createElement('m'); b.appendChild(m); \
         m.before('x');",
    )
    .unwrap();
    ok(
        &mut rt,
        "b.childNodes.length === 3 && b.firstChild === a \
         && a.nextSibling.data === 'x' && a.nextSibling.nextSibling === m",
    );
}

/// `previousElementSibling` / `nextElementSibling` at a tree boundary (no
/// preceding or following sibling at all, element or otherwise): the
/// search loop never runs, falling straight through to `null`.
#[test]
fn element_sibling_getters_are_null_at_a_boundary() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; var x = document.createElement('x'); b.appendChild(x);")
        .unwrap();
    ok(
        &mut rt,
        "x.previousElementSibling === null && x.nextElementSibling === null",
    );
}

/// `Node.nodeValue`'s setter is a no-op for every interface but
/// `CharacterData` (DOM §4.4: "Otherwise: do nothing"), exercised here
/// against an `Element`, unlike the getter-only check elsewhere.
#[test]
fn node_value_setter_is_a_no_op_off_character_data() {
    let mut rt = rt();
    rt.evaluate("document.body.nodeValue = 'ignored';").unwrap();
    ok(&mut rt, "document.body.nodeValue === null");
}

/// `CharacterData.length` counts UTF-16 code units (DOM §4.10), not
/// raikiri-dom's UTF-8 byte length: an astral character is one Unicode
/// scalar value but a two-unit UTF-16 surrogate pair.
#[test]
fn character_data_length_counts_utf16_code_units() {
    let mut rt = rt();
    rt.evaluate("var t = document.createTextNode('a\\u{1F600}b');")
        .unwrap();
    ok(&mut rt, "t.length === 4");
}

/// The `textContent` setter's DocumentFragment/Element branch clears every
/// child for an empty string, rather than inserting an empty Text node.
#[test]
fn fragment_text_content_setter_empty_string_clears_children() {
    let mut rt = rt();
    rt.evaluate(
        "var f = document.createDocumentFragment(); \
         f.append(document.createElement('a'), 'x'); \
         f.textContent = '';",
    )
    .unwrap();
    ok(&mut rt, "f.childNodes.length === 0");
}

/// `ParentNode.append`'s node-or-string arguments are converted and spliced
/// into a temporary fragment one at a time; a validity failure partway
/// through (here, the Document root cannot be inserted as a child of
/// anything) must still surface as the same `DOMException` any other
/// pre-insert failure would.
#[test]
fn append_wrapping_multiple_nodes_propagates_a_validity_error() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { document.body.append(document.createElement('a'), document); false } \
         catch (e) { e.name === 'HierarchyRequestError' }",
    );
}
