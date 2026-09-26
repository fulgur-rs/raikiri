use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;

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

#[test]
fn child_nodes_and_children_are_live_and_stable() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; var cn = b.childNodes; var ch = b.children; b.append('t', document.createElement('e'));").unwrap();
    ok(
        &mut rt,
        "cn === b.childNodes && ch === b.children && cn instanceof NodeList && ch instanceof HTMLCollection",
    );
    ok(
        &mut rt,
        "cn.length === 2 && ch.length === 1 && ch[0].localName === 'e' && cn.item(0).data === 't' && cn[5] === undefined && cn.item(5) === null",
    );
    rt.evaluate("b.removeChild(b.firstChild);").unwrap();
    ok(&mut rt, "cn.length === 1 && cn[0] === ch[0]");
}

#[test]
fn query_selector_all_is_static() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; b.appendChild(document.createElement('p')); var q = document.querySelectorAll('p'); b.appendChild(document.createElement('p'));").unwrap();
    ok(
        &mut rt,
        "q instanceof NodeList && q.length === 1 && document.querySelectorAll('p').length === 2",
    );
}

#[test]
fn get_elements_by_is_live() {
    let mut rt = rt();
    rt.evaluate("var g = document.getElementsByTagName('i'); document.body.appendChild(document.createElement('i'));").unwrap();
    ok(&mut rt, "g instanceof HTMLCollection && g.length === 1");
}

#[test]
fn iteration_and_index_semantics() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; b.append(document.createElement('a'), document.createElement('b'));").unwrap();
    ok(
        &mut rt,
        "var n = []; for (var x of b.children) n.push(x.localName); n.join() === 'a,b'",
    );
    ok(
        &mut rt,
        "var m = []; b.childNodes.forEach(function (x, i) { m.push(i); }); m.join() === '0,1'",
    );
    ok(
        &mut rt,
        "Object.keys(b.children).join() === '0,1' && ('1' in b.children) && !('2' in b.children)",
    );
    ok(
        &mut rt,
        "Array.prototype.slice.call(b.childNodes).length === 2",
    );
    ok(
        &mut rt,
        "try { NodeList.prototype.item.call({}, 0); false } catch (e) { e instanceof TypeError }",
    );
    ok(&mut rt, "b.children.namedItem('nope') === null");
}

#[test]
fn interface_objects_and_iteration_members() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { new NodeList(); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { new HTMLCollection(); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "NodeList.prototype.forEach === Array.prototype.forEach \
         && NodeList.prototype.entries === Array.prototype.entries \
         && NodeList.prototype.keys === Array.prototype.keys \
         && NodeList.prototype.values === Array.prototype.values \
         && NodeList.prototype[Symbol.iterator] === Array.prototype.values \
         && HTMLCollection.prototype[Symbol.iterator] === Array.prototype.values \
         && !('forEach' in HTMLCollection.prototype)",
    );
    ok(
        &mut rt,
        "var d = Object.getOwnPropertyDescriptor(NodeList.prototype, 'forEach'); \
         d.writable && d.enumerable && d.configurable",
    );
    ok(
        &mut rt,
        "var d = Object.getOwnPropertyDescriptor(HTMLCollection.prototype, Symbol.iterator); \
         d.writable && !d.enumerable && d.configurable",
    );
    rt.evaluate("var b = document.body; b.append('x', document.createElement('y'));")
        .unwrap();
    ok(
        &mut rt,
        "var e = []; for (var p of b.childNodes.entries()) e.push(p[0]); \
         var k = Array.from(b.childNodes.keys()); var v = Array.from(b.childNodes.values()); \
         e.join() === '0,1' && k.join() === '0,1' && v[1].localName === 'y'",
    );
}

#[test]
fn brand_checks_reject_other_this_values() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; b.append(document.createElement('a'));")
        .unwrap();
    for src in [
        "NodeList.prototype.item.call(b.children, 0)",
        "Object.getOwnPropertyDescriptor(NodeList.prototype, 'length').get.call({})",
        "Object.getOwnPropertyDescriptor(NodeList.prototype, 'length').get.call(1)",
        "HTMLCollection.prototype.item.call(b.childNodes, 0)",
        "Object.getOwnPropertyDescriptor(HTMLCollection.prototype, 'length').get.call(b.childNodes)",
        "HTMLCollection.prototype.namedItem.call(b.childNodes, 'a')",
        "NodeList.prototype.item.call(new Proxy(b.childNodes, {}), 0)",
        "b.childNodes.item()",
        "b.children.item()",
        "b.children.namedItem()",
    ] {
        ok(
            &mut rt,
            &format!("try {{ {src}; false }} catch (e) {{ e instanceof TypeError }}"),
        );
    }
    ok(
        &mut rt,
        "b.children.item(0) === b.firstChild && b.children.item(1) === null \
         && b.childNodes.item(-1) === null && b.childNodes.item('0') === b.firstChild",
    );
}

#[test]
fn index_keys_are_canonical_array_indices_only() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; b.append(document.createElement('a'), document.createElement('c')); var c = b.childNodes;").unwrap();
    ok(
        &mut rt,
        "c['01'] === undefined && c['-0'] === undefined && c['1.0'] === undefined \
         && !('01' in c) && !('-0' in c) && !('1.0' in c) && ('0' in c) && (1 in c) && c[1] === c.item(1)",
    );
    ok(
        &mut rt,
        "'item' in c && 'length' in c && !('nope' in c) && c.nope === undefined",
    );
}

#[test]
fn own_property_protocol_matches_legacy_platform_objects() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; b.append(document.createElement('a'), document.createElement('c')); var c = b.childNodes;").unwrap();
    ok(
        &mut rt,
        "var d = Object.getOwnPropertyDescriptor(c, '0'); \
         d.value === b.firstChild && !d.writable && d.enumerable && d.configurable \
         && Object.getOwnPropertyDescriptor(c, '2') === undefined \
         && Object.getOwnPropertyDescriptor(c, 'length') === undefined",
    );
    // Expandos and symbols live on the target, after the indices.
    ok(
        &mut rt,
        "var s = Symbol('s'); c.foo = 1; c[s] = 2; \
         var keys = Reflect.ownKeys(c); \
         keys.length === 4 && keys[0] === '0' && keys[1] === '1' && keys[2] === 'foo' && keys[3] === s \
         && c.foo === 1 && c[s] === 2 && Object.getOwnPropertyDescriptor(c, 'foo').value === 1 \
         && delete c.foo && !('foo' in c)",
    );
    // No indexed setter: writes are ignored (sloppy) or throw (strict).
    ok(&mut rt, "var f = b.firstChild; c[0] = 1; c[0] === f");
    ok(
        &mut rt,
        "(function () { 'use strict'; try { c[0] = 1; return false } catch (e) { return e instanceof TypeError } })()",
    );
    ok(
        &mut rt,
        "(function () { 'use strict'; try { c[7] = 1; return false } catch (e) { return e instanceof TypeError } })() && c[7] === undefined",
    );
    ok(
        &mut rt,
        "try { Object.defineProperty(c, '0', { value: 1 }); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "Object.defineProperty(c, 'bar', { value: 3 }) === c && c.bar === 3",
    );
    // A supported index cannot be deleted; an unsupported one can.
    ok(
        &mut rt,
        "(function () { 'use strict'; try { delete c[0]; return false } catch (e) { return e instanceof TypeError } })()",
    );
    ok(&mut rt, "delete c[9] && c.length === 2");
    // The target stays extensible, so the index descriptors stay valid.
    ok(
        &mut rt,
        "try { Object.preventExtensions(c); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { Object.freeze(c); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "Object.isExtensible(c) && Reflect.preventExtensions(c) === false && c[0] === b.firstChild",
    );
    ok(&mut rt, "Object.getPrototypeOf(c) === NodeList.prototype");
}

#[test]
fn named_item_matches_id_then_name() {
    let mut rt = rt();
    rt.evaluate(
        "var b = document.body; var x = document.createElement('x'); var y = document.createElement('y'); \
         x.setAttribute('name', 'n'); y.id = 'n'; x.id = 'i'; b.append(x, y);",
    )
    .unwrap();
    ok(
        &mut rt,
        "b.children.namedItem('n') === x && b.children.namedItem('i') === x \
         && b.children.namedItem('') === null && b.children.namedItem('zz') === null",
    );
}

#[test]
fn get_elements_by_class_name_is_live() {
    let mut rt = rt();
    rt.evaluate("var g = document.getElementsByClassName(' a  b '); var e = document.getElementsByClassName(' '); var el = document.createElement('p'); el.className = 'b a'; document.body.appendChild(el);").unwrap();
    ok(
        &mut rt,
        "g instanceof HTMLCollection && g.length === 1 && g[0] === el && e.length === 0",
    );
    rt.evaluate("el.className = 'a';").unwrap();
    ok(&mut rt, "g.length === 0");
    ok(
        &mut rt,
        "var s = document.body.getElementsByTagName('p'); s.length === 1 && document.getElementsByTagName('p') !== document.getElementsByTagName('p')",
    );
}

#[test]
fn child_nodes_of_every_node_kind() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var t = document.createTextNode('t'); t.childNodes.length === 0 && t.childNodes === t.childNodes \
         && document.children.length === 1 && document.children[0] === document.documentElement",
    );
}
