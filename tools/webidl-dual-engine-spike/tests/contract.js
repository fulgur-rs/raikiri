(() => {
    "use strict";
    const results = [];
    function test(name, body) {
        try { body(); results.push({ name, status: "PASS" }); }
        catch (e) { results.push({ name, status: "FAIL", error: String(e) }); }
    }
    function eq(actual, expected) {
        if (!Object.is(actual, expected)) throw new Error(`expected ${String(expected)}, got ${String(actual)}`);
    }
    function throwsTypeError(body) {
        try { body(); } catch (e) { if (e instanceof TypeError) return; throw e; }
        throw new Error("expected TypeError");
    }
    test("inheritance", () => { eq(element instanceof Element, true); eq(element instanceof Node, true); });
    test("prototype chain", () => { eq(Object.getPrototypeOf(Element.prototype), Node.prototype); eq(Object.getPrototypeOf(Element), Node); });
    test("node type", () => { eq(element.nodeType, 1); eq(documentNode.nodeType, 9); });
    test("parent identity", () => { eq(element.parentNode, documentNode); eq(element.parentNode, element.parentNode); });
    test("wrapper identity after GC", () => { const parent = element.parentNode; collectGarbage(); eq(element.parentNode, parent); eq(element.nodeType, 1); });
    test("null parent", () => eq(documentNode.parentNode, null));
    test("same node", () => { eq(element.isSameNode(element), true); eq(element.isSameNode(documentNode), false); });
    test("nullable node argument", () => { eq(element.isSameNode(null), false); eq(element.isSameNode(undefined), false); });
    test("node argument brand", () => throwsTypeError(() => element.isSameNode({})));
    test("missing node argument", () => throwsTypeError(() => element.isSameNode()));
    test("illegal constructors", () => { throwsTypeError(() => new Node()); throwsTypeError(() => new Element()); });
    test("get missing attribute", () => eq(element.getAttribute("absent"), null));
    test("set then get", () => { eq(element.setAttribute("data-x", "hello"), undefined); eq(element.getAttribute("data-x"), "hello"); });
    test("empty attribute", () => { element.setAttribute("data-empty", ""); eq(element.getAttribute("data-empty"), ""); });
    test("HTML attribute case folding", () => { element.setAttribute("DATA-CASE", "yes"); eq(element.getAttribute("data-case"), "yes"); eq(element.getAttribute("DATA-CASE"), "yes"); });
    test("DOMString null", () => { element.setAttribute("data-null", null); eq(element.getAttribute("data-null"), "null"); });
    test("DOMString undefined", () => { element.setAttribute("data-undefined", undefined); eq(element.getAttribute("data-undefined"), "undefined"); });
    test("DOMString number", () => { element.setAttribute("data-number", 42); eq(element.getAttribute("data-number"), "42"); });
    test("DOMString object", () => { element.setAttribute("data-object", { toString() { return "converted"; } }); eq(element.getAttribute("data-object"), "converted"); });
    test("DOMString symbol", () => throwsTypeError(() => element.setAttribute("data-symbol", Symbol("s"))));
    test("DOMString conversion order", () => {
        const order = [];
        element.setAttribute({ toString() { order.push("name"); return "data-order"; } }, { toString() { order.push("value"); return "v"; } });
        eq(order.join(","), "name,value");
    });
    test("conversion reentrancy", () => {
        element.setAttribute("data-outer", { toString() { element.setAttribute("data-inner", "nested"); return "outer"; } });
        eq(element.getAttribute("data-inner"), "nested"); eq(element.getAttribute("data-outer"), "outer");
    });
    test("conversion exception identity", () => {
        const error = new Error("sentinel");
        try { element.setAttribute("data-exception", { toString() { throw error; } }); }
        catch (e) { eq(e, error); eq(element.getAttribute("data-exception"), null); return; }
        throw new Error("exception was swallowed");
    });
    test("missing get argument", () => throwsTypeError(() => element.getAttribute()));
    test("missing set argument", () => { throwsTypeError(() => element.setAttribute("data-missing")); eq(element.getAttribute("data-missing"), null); });
    test("extra arguments ignored", () => { element.setAttribute("data-extra", "ok", Symbol()); eq(element.getAttribute("data-extra"), "ok"); });
    test("method receiver brand", () => throwsTypeError(() => Element.prototype.getAttribute.call({}, "x")));
    test("prototype forged receiver", () => throwsTypeError(() => Element.prototype.getAttribute.call(Object.create(Element.prototype), "x")));
    test("subtype receiver brand", () => throwsTypeError(() => Element.prototype.getAttribute.call(documentNode, "x")));
    test("getter receiver brand", () => throwsTypeError(() => Object.getOwnPropertyDescriptor(Node.prototype, "nodeType").get.call({})));
    test("proxy receiver brand", () => throwsTypeError(() => Element.prototype.getAttribute.call(new Proxy(element, {}), "x")));
    test("brand before conversion", () => { let called = false; throwsTypeError(() => Element.prototype.getAttribute.call({}, { toString() { called = true; return "x"; } })); eq(called, false); });
    test("method descriptor", () => { const d = Object.getOwnPropertyDescriptor(Element.prototype, "setAttribute"); eq(d.writable, true); eq(d.enumerable, true); eq(d.configurable, true); eq(d.value.length, 2); eq(d.value.name, "setAttribute"); });
    test("getter descriptor", () => { const d = Object.getOwnPropertyDescriptor(Node.prototype, "nodeType"); eq(d.enumerable, true); eq(d.configurable, true); eq(d.set, undefined); eq(d.get.name, "get nodeType"); });
    test("readonly attribute", () => { throwsTypeError(() => { element.nodeType = 99; }); eq(element.nodeType, 1); });
    test("toStringTag", () => { eq(Object.prototype.toString.call(element), "[object Element]"); eq(Object.prototype.toString.call(documentNode), "[object Node]"); });
    test("valid Unicode", () => { element.setAttribute("data-unicode", "日本語😀\u0000"); eq(element.getAttribute("data-unicode"), "日本語😀\u0000"); });
    test("remove attribute", () => { eq(element.removeAttribute("data-x"), undefined); eq(element.getAttribute("data-x"), null); });
    test("native DOM error", () => {
        let caught;
        try { element.setAttribute("", "x"); } catch (e) { caught = e; }
        eq(caught.name, "InvalidCharacterError");
        eq(caught instanceof Error, true);
    });
    test("native error ignores inherited setters", () => {
        const original = Object.getOwnPropertyDescriptor(Error.prototype, "name");
        let calls = 0;
        try {
            Object.defineProperty(Error.prototype, "name", { set() { calls++; throw new Error("poisoned name"); }, configurable: true });
            let caught;
            try { element.setAttribute("", "x"); } catch (e) { caught = e; }
            eq(calls, 0); eq(caught.name, "InvalidCharacterError");
        } finally { Object.defineProperty(Error.prototype, "name", original); }
    });
    test("native error overrides inherited readonly name", () => {
        const original = Object.getOwnPropertyDescriptor(Error.prototype, "name");
        try {
            Object.defineProperty(Error.prototype, "name", { value: "poisoned", writable: false, configurable: true });
            let caught;
            try { element.setAttribute("", "x"); } catch (e) { caught = e; }
            eq(caught.name, "InvalidCharacterError");
        } finally { Object.defineProperty(Error.prototype, "name", original); }
    });
    test("real DOM mutation", () => element.setAttribute("data-final", "from-js"));
    return JSON.stringify(results);
})();
