use boa_engine::JsString;
use boa_engine::property::Attribute;

use crate::runtime::DomRuntime;
use crate::runtime::interfaces::wrap;
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;

fn rt() -> DomRuntime {
    let (h, ..) = StubHost::page();
    DomRuntime::new(h).unwrap()
}

/// [`rt`] plus the fixture page's `<body>` arena index, for tests that build
/// nodes directly against the document.
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

const TREE: &str = "var b = document.body; b.innerHTML = '';
  var s = document.createElement('section'); s.className = 'a b'; s.id = 'sec';
  var p1 = document.createElement('p'); p1.className = 'a';
  var p2 = document.createElement('p'); p2.className = 'b a';
  var sp = document.createElement('span');
  p2.appendChild(sp); s.append(p1, p2); b.appendChild(s);";

#[test]
fn element_scoped_queries() {
    let mut rt = rt();
    rt.evaluate(TREE).unwrap();
    ok(
        &mut rt,
        "s.querySelector('p') === p1 && s.querySelector('span') === sp && s.querySelector('section') === null",
    );
    ok(
        &mut rt,
        "var all = s.querySelectorAll('p'); all.length === 2 && all[0] === p1 && all[1] === p2",
    );
    ok(&mut rt, "document.querySelectorAll('.a').length === 3");
    ok(
        &mut rt,
        "sp.matches('p > span') && !sp.matches('section > span') && sp.closest('section') === s && sp.closest('span') === sp && sp.closest('table') === null",
    );
    ok(
        &mut rt,
        "try { sp.matches('['); false } catch (e) { e.name === 'SyntaxError' }",
    );
}

/// A scoped query restricts *candidates* to the subtree, but combinator
/// matching still sees the whole document tree: `body` is a real ancestor
/// of `s` even though `s` is the scoping root, so a selector naming an
/// ancestor outside the scoped subtree still matches.
#[test]
fn scoped_query_selector_sees_real_ancestors_above_the_scoping_root() {
    let mut rt = rt();
    rt.evaluate(TREE).unwrap();
    ok(&mut rt, "s.querySelector('body p') === p1");
    ok(&mut rt, "s.querySelectorAll('html span').length === 1");
}

#[test]
fn matches_works_on_detached_elements() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var d = document.createElement('div'); var i = document.createElement('i'); d.appendChild(i); i.matches('div > i') && i.closest('div') === d",
    );
}

#[test]
fn get_elements_by_tag_and_class_name() {
    let mut rt = rt();
    rt.evaluate(TREE).unwrap();
    ok(
        &mut rt,
        "document.getElementsByTagName('P').length === 2 && s.getElementsByTagName('*').length === 3",
    );
    ok(
        &mut rt,
        "document.getElementsByClassName('a b').length === 2 && document.getElementsByClassName('b')[0] === s",
    );
    ok(&mut rt, "document.getElementsByClassName('').length === 0");
    ok(&mut rt, "s.getElementsByClassName('a').length === 2");
}

/// DOM §4.4 "HTML namespace + HTML document" tag-name matching only
/// ASCII-lowercases the comparison for HTML-namespace elements; a
/// non-HTML-namespace element (e.g. a foreign SVG element) compares its
/// qualified name case-sensitively.
#[test]
fn get_elements_by_tag_name_is_case_sensitive_outside_the_html_namespace() {
    let (mut rt, body) = rt_with_body();
    with_state(rt.context_mut(), |s| {
        let doc = s.host.document_mut();
        let svg = doc.create_detached_element("svg").unwrap();
        doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
        doc.append_child(body, svg).unwrap();
    })
    .unwrap();
    ok(
        &mut rt,
        "document.getElementsByTagName('svg').length === 1 && document.getElementsByTagName('SVG').length === 0",
    );
}

#[test]
fn id_class_name_and_attribute_names() {
    let mut rt = rt();
    rt.evaluate(TREE).unwrap();
    ok(
        &mut rt,
        "s.id === 'sec' && s.getAttribute('id') === 'sec' && p2.className === 'b a'",
    );
    rt.evaluate("s.id = 'x';").unwrap();
    ok(
        &mut rt,
        "document.getElementById('x') === s && document.getElementById('sec') === null",
    );
    ok(
        &mut rt,
        "var n = s.getAttributeNames(); n.length === 2 && n.indexOf('class') >= 0 && n.indexOf('id') >= 0",
    );
}

#[test]
fn fragment_queries() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var f = document.createDocumentFragment(); var q = document.createElement('q'); q.id = 'qq'; f.appendChild(q); f.querySelector('q') === q && f.getElementById('qq') === q && f.querySelectorAll('*').length === 1",
    );
}

/// `getElementById` is `NonElementParentNode` (Document, DocumentFragment
/// only); calling it on an `Element` must fail the brand check rather than
/// silently returning `null`.
#[test]
fn get_element_by_id_is_not_available_on_element() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { DocumentFragment.prototype.getElementById.call(document.body, 'x'); false } catch (e) { e instanceof TypeError }",
    );
}

/// CSS Selectors L4 `:scope` (§14.3.3): an `Element`-scoped
/// `querySelector`/`querySelectorAll` binds its own scoping root as
/// `:scope`; `Element.matches`/`closest` bind `this`.
#[test]
fn scope_pseudo_class_binds_to_the_scoping_element() {
    let mut rt = rt();
    rt.evaluate(TREE).unwrap();
    ok(&mut rt, "s.querySelector(':scope > p') === p1");
    ok(&mut rt, "s.querySelectorAll(':scope > p').length === 2");
    // `Element.matches` always binds `:scope` to `this` itself, so
    // `x.matches(':scope')` is trivially true for any `x` -- the
    // meaningful check is that `:scope` still combines correctly with the
    // rest of the selector: `p1`'s own parent (`s`) is not `p1`, so a
    // selector requiring an ancestor that *is* the scope fails.
    ok(&mut rt, "s.matches(':scope')");
    ok(&mut rt, "!p1.matches(':scope > p')");
    ok(&mut rt, "sp.closest(':scope') === sp");
}

/// A Document/DocumentFragment-scoped query has no element to bind
/// `:scope` to; it then falls back to `:root` semantics (matches only the
/// document element).
#[test]
fn scope_pseudo_class_falls_back_to_root_on_a_document_scoped_query() {
    let mut rt = rt();
    ok(
        &mut rt,
        "document.querySelector(':scope') === document.documentElement",
    );
    ok(
        &mut rt,
        "document.documentElement.closest('html') === document.documentElement",
    );
}

/// **Known limitation** (see this module's doc): a sibling combinator's
/// candidate lookup consults raikiri-dom's `IS_IN_DOCUMENT` flag directly,
/// which stays clear for every node of a tree that was never attached to
/// the real document -- including a `DocumentFragment`'s own contents.
/// `b` really is `i`'s immediately preceding sibling here, but `b + i`
/// still fails to match because neither is ever marked in-document. This
/// test pins the current (incorrect) behavior rather than asserting it is
/// correct.
#[test]
fn matches_under_matches_a_sibling_combinator_on_a_fragment_child() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var f = document.createDocumentFragment();
         var b = document.createElement('b'); var i = document.createElement('i');
         f.append(b, i);
         i.matches('b + i') === false",
    );
}

/// Descendant/child combinators are unaffected by the limitation above --
/// they walk this binding's own always-accurate ancestor chain, never the
/// `IS_IN_DOCUMENT` flag, so they still match correctly inside a
/// `DocumentFragment`'s detached contents.
#[test]
fn matches_descendant_combinator_works_on_a_fragment_child() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var f = document.createDocumentFragment();
         var d = document.createElement('div'); var sp = document.createElement('span');
         d.appendChild(sp); f.appendChild(d);
         sp.matches('div span')",
    );
}

#[test]
fn query_selector_empty_string_is_a_syntax_error() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { document.querySelector(''); false } catch (e) { e.name === 'SyntaxError' }",
    );
}

fn is_dirty(rt: &mut DomRuntime) -> bool {
    with_state(rt.context_mut(), |s| s.dirty).unwrap()
}

fn clear_dirty(rt: &mut DomRuntime) {
    with_state(rt.context_mut(), |s| s.dirty = false).unwrap();
}

#[test]
fn id_and_class_name_setters_mark_dirty() {
    let mut rt = rt();
    rt.evaluate("var d = document.createElement('div'); document.body.appendChild(d);")
        .unwrap();
    clear_dirty(&mut rt);
    rt.evaluate("d.id = 'x';").unwrap();
    assert!(is_dirty(&mut rt), "id= should mark dirty");

    clear_dirty(&mut rt);
    rt.evaluate("d.className = 'y';").unwrap();
    assert!(is_dirty(&mut rt), "className= should mark dirty");
}
