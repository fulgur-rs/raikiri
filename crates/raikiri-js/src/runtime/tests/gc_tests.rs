use super::{DomRuntime, StubHost, eval_bool};
use crate::runtime::webidl::with_state;

fn runtime() -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

fn collect(runtime: &mut DomRuntime) {
    runtime.context_mut().clear_kept_objects();
    boa_gc::force_collect();
}

#[test]
fn discarded_detached_node_and_its_payload_are_collectible() {
    let mut rt = runtime();
    rt.evaluate(
        "var plain = new WeakRef({}); var nodeWeak; var payloadWeak; \
         (function () { \
             var node = document.createElement('div'); \
             node.payload = new Uint8Array(1048576); \
             nodeWeak = new WeakRef(node); payloadWeak = new WeakRef(node.payload); \
         })();",
    )
    .unwrap();
    collect(&mut rt);
    assert!(eval_bool(&mut rt, "plain.deref() === undefined"));
    assert!(eval_bool(&mut rt, "nodeWeak.deref() === undefined"));
    assert!(eval_bool(&mut rt, "payloadWeak.deref() === undefined"));
}

#[test]
fn connected_node_identity_and_expando_survive_collection() {
    let mut rt = runtime();
    rt.evaluate(
        "var connectedWeak; var connectedPayload; \
         (function () { \
             var node = document.createElement('div'); node.id = 'connected'; \
             node.payload = { value: 42 }; document.body.appendChild(node); \
             connectedWeak = new WeakRef(node); connectedPayload = new WeakRef(node.payload); \
         })();",
    )
    .unwrap();
    collect(&mut rt);
    assert!(eval_bool(
        &mut rt,
        "document.getElementById('connected') === connectedWeak.deref() \
         && connectedWeak.deref().payload === connectedPayload.deref() \
         && connectedPayload.deref().value === 42"
    ));
}

#[test]
fn detached_component_survives_through_a_child_then_is_collectible() {
    let mut rt = runtime();
    rt.evaluate(
        "var retainedChild; var parentWeak; var siblingWeak; \
         (function () { \
             var parent = document.createElement('div'); parent.payload = 11; \
             retainedChild = document.createElement('span'); \
             var sibling = document.createElement('p'); sibling.payload = 22; \
             parent.append(retainedChild, sibling); \
             parentWeak = new WeakRef(parent); siblingWeak = new WeakRef(sibling); \
         })();",
    )
    .unwrap();
    collect(&mut rt);
    assert!(eval_bool(
        &mut rt,
        "retainedChild.parentNode === parentWeak.deref() \
         && retainedChild.parentNode.payload === 11 \
         && retainedChild.nextSibling === siblingWeak.deref() \
         && retainedChild.nextSibling.payload === 22"
    ));
    let child = crate::runtime::webidl::node_index(&rt.evaluate("retainedChild").unwrap()).unwrap();
    rt.evaluate("retainedChild = null;").unwrap();
    collect(&mut rt);
    assert!(eval_bool(&mut rt, "parentWeak.deref() === undefined"));
    assert!(eval_bool(&mut rt, "siblingWeak.deref() === undefined"));
    let object = crate::runtime::interfaces::wrap(rt.context_mut(), child).unwrap();
    rt.context_mut()
        .register_global_property(
            boa_engine::JsString::from("restoredChild"),
            object,
            boa_engine::property::Attribute::all(),
        )
        .unwrap();
    assert!(eval_bool(
        &mut rt,
        "restoredChild.parentNode.tagName === 'DIV' && restoredChild.nextSibling.tagName === 'P' && restoredChild.parentNode.payload === undefined"
    ));
}

#[test]
fn native_reattachment_restores_template_contents_after_host_collection() {
    let (host, _, _, body) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let (outer, child) = with_state(rt.context_mut(), |state| {
        let doc = state.host.document_mut();
        let outer = doc.create_detached_element("div").unwrap();
        doc.set_element_attribute(outer, "id", "native-outer")
            .unwrap();
        let template = doc.append_element(Some(outer), "template", None::<&str>);
        let contents = doc.allocate_template_fragment_root(template);
        let child = doc.append_element(Some(contents), "span", None::<&str>);
        (outer, child)
    })
    .unwrap();
    let object = crate::runtime::interfaces::wrap(rt.context_mut(), outer).unwrap();
    rt.context_mut()
        .register_global_property(
            boa_engine::JsString::from("nativeOuter"),
            object,
            boa_engine::property::Attribute::all(),
        )
        .unwrap();
    let object = crate::runtime::interfaces::wrap(rt.context_mut(), child).unwrap();
    rt.context_mut()
        .register_global_property(
            boa_engine::JsString::from("heldContent"),
            object,
            boa_engine::property::Attribute::all(),
        )
        .unwrap();
    rt.evaluate(
        "heldContent.payload = { value: 99 }; \
         var contentWeak = new WeakRef(heldContent); \
         var outerWeak = new WeakRef(nativeOuter); nativeOuter = null;",
    )
    .unwrap();
    collect(&mut rt);
    assert!(eval_bool(&mut rt, "outerWeak.deref() === undefined"));
    with_state(rt.context_mut(), |state| {
        let doc = state.host.document_mut();
        doc.pre_insert(body, outer, None).unwrap();
    })
    .unwrap();
    rt.evaluate("heldContent = null;").unwrap();
    collect(&mut rt);
    let object = crate::runtime::interfaces::wrap(rt.context_mut(), child).unwrap();
    rt.context_mut()
        .register_global_property(
            boa_engine::JsString::from("reexposedContent"),
            object,
            boa_engine::property::Attribute::all(),
        )
        .unwrap();
    assert!(eval_bool(
        &mut rt,
        "reexposedContent === contentWeak.deref() && contentWeak.deref().payload.value === 99"
    ));
}

#[test]
fn attribute_mutations_do_not_walk_ownership_edges() {
    let mut rt = runtime();
    let before = with_state(rt.context_mut(), |state| state.node_graph.edge_visits).unwrap();
    rt.evaluate(
        "for (var i = 0; i < 1000; i++) { document.body.setAttribute('data-count', String(i)); }",
    )
    .unwrap();
    let after = with_state(rt.context_mut(), |state| state.node_graph.edge_visits).unwrap();
    assert_eq!(after, before);
}

#[test]
fn appending_nodes_updates_a_linear_number_of_ownership_edges() {
    let mut rt = runtime();
    let before = with_state(rt.context_mut(), |state| state.node_graph.edge_visits).unwrap();
    rt.evaluate(
        "for (var i = 0; i < 256; i++) { document.body.appendChild(document.createElement('div')); }",
    )
    .unwrap();
    let after = with_state(rt.context_mut(), |state| state.node_graph.edge_visits).unwrap();
    assert!(after - before <= 8 * 256);
}

#[test]
fn cached_side_object_cycles_are_collectible() {
    let fixtures = [
        (
            "style",
            "var node = document.createElement('div'); var view = node.style;",
        ),
        (
            "computed style",
            "var node = document.createElement('div'); var view = getComputedStyle(node);",
        ),
        (
            "classList",
            "var node = document.createElement('div'); var view = node.classList;",
        ),
        (
            "childNodes",
            "var node = document.createElement('div'); var view = node.childNodes;",
        ),
        (
            "children",
            "var node = document.createElement('div'); var view = node.children;",
        ),
        (
            "canvas",
            "var node = document.createElement('canvas'); var view = node.getContext('2d');",
        ),
        (
            "stylesheet",
            "var node = document.createElement('style'); node.textContent = 'p { color:red }'; document.body.appendChild(node); var view = node.sheet; node.remove();",
        ),
        (
            "rule",
            "var node = document.createElement('style'); node.textContent = 'p { color:red }'; document.body.appendChild(node); var view = node.sheet.cssRules[0]; node.remove();",
        ),
    ];
    let mut retained = Vec::new();
    for (name, fixture) in fixtures {
        let mut rt = runtime();
        rt.evaluate(&format!(
            "var nodeWeak; var viewWeak; (function () {{ {fixture} \
             view.owner = node; nodeWeak = new WeakRef(node); viewWeak = new WeakRef(view); }})();"
        ))
        .unwrap();
        collect(&mut rt);
        if !eval_bool(
            &mut rt,
            "nodeWeak.deref() === undefined && viewWeak.deref() === undefined",
        ) {
            retained.push(name);
        }
    }
    assert!(
        retained.is_empty(),
        "retained side-object families: {retained:?}"
    );
}

#[test]
fn listener_self_cycles_are_collectible() {
    let mut rt = runtime();
    rt.evaluate(
        "var listenerNodeWeak; (function () { \
             var node = document.createElement('div'); \
             node.addEventListener('test', function () { return node; }); \
             listenerNodeWeak = new WeakRef(node); \
         })();",
    )
    .unwrap();
    collect(&mut rt);
    assert!(eval_bool(&mut rt, "listenerNodeWeak.deref() === undefined"));
}

#[test]
fn auxiliary_objects_retain_their_nodes_until_released() {
    let fixtures = [
        ("style", "held = node.style;"),
        ("computed style", "held = getComputedStyle(node);"),
        ("classList", "held = node.classList;"),
        ("childNodes", "held = node.childNodes;"),
        ("children", "held = node.children;"),
        (
            "static NodeList",
            "var parent = document.createElement('div'); parent.appendChild(node); held = parent.querySelectorAll('div'); node.remove();",
        ),
        (
            "Range",
            "held = document.createRange(); held.selectNodeContents(node);",
        ),
        ("TreeWalker", "held = document.createTreeWalker(node);"),
        (
            "TreeWalker external currentNode",
            "held = document.createTreeWalker(document.body); held.currentNode = node;",
        ),
        (
            "Event",
            "held = new Event('test'); node.dispatchEvent(held);",
        ),
        (
            "FocusEvent",
            "held = new FocusEvent('focus', {relatedTarget: node});",
        ),
        (
            "Animation",
            "held = node.animate([{width:'0px'}, {width:'10px'}], 1000);",
        ),
    ];
    let mut lost = Vec::new();
    let mut leaked = Vec::new();
    for (name, fixture) in fixtures {
        let mut rt = runtime();
        rt.evaluate(&format!(
            "var held; var ownerWeak; var valueWeak; (function () {{ \
             var node = document.createElement('div'); node.payload = {{value: 47}}; \
             ownerWeak = new WeakRef(node); valueWeak = new WeakRef(node.payload); {fixture} }})();"
        ))
        .unwrap();
        collect(&mut rt);
        if !eval_bool(
            &mut rt,
            "ownerWeak.deref() !== undefined && ownerWeak.deref().payload === valueWeak.deref() && valueWeak.deref().value === 47",
        ) {
            lost.push(name);
        }
        rt.evaluate("held = null;").unwrap();
        collect(&mut rt);
        if !eval_bool(
            &mut rt,
            "ownerWeak.deref() === undefined && valueWeak.deref() === undefined",
        ) {
            leaked.push(name);
        }
    }
    assert!(
        lost.is_empty() && leaked.is_empty(),
        "lost owners: {lost:?}; leaked owners: {leaked:?}"
    );
}

#[test]
fn detached_page_rules_keep_margin_identity_and_release_the_original_node() {
    let mut rt = runtime();
    rt.evaluate(
        "var heldPage; var heldStyle; var marginWeak; var cssNodeWeak; (function () { \
        var node = document.createElement('style'); \
        node.textContent = '@page { @top-left { content: \"hello\" } }'; \
        document.body.appendChild(node); var sheet = node.sheet; \
        heldPage = sheet.cssRules[0]; var margin = heldPage.cssRules[0]; \
        margin.payload = {value: 63}; marginWeak = new WeakRef(margin); \
        heldStyle = margin.style; cssNodeWeak = new WeakRef(node); \
        sheet.deleteRule(0); node.remove(); \
    })();",
    )
    .unwrap();
    collect(&mut rt);
    assert!(eval_bool(
        &mut rt,
        "cssNodeWeak.deref() === undefined && heldPage.cssRules[0] === marginWeak.deref() && marginWeak.deref().payload.value === 63"
    ));
    rt.evaluate("heldPage = null;").unwrap();
    collect(&mut rt);
    assert!(eval_bool(
        &mut rt,
        "marginWeak.deref() !== undefined && heldStyle === marginWeak.deref().style"
    ));
    rt.evaluate("heldStyle = null;").unwrap();
    collect(&mut rt);
    assert!(eval_bool(&mut rt, "marginWeak.deref() === undefined"));
}

#[test]
fn live_collections_do_not_retain_removed_members() {
    let mut rt = runtime();
    rt.evaluate(
        "var heldList; var removedWeak; (function () { \
        var parent = document.createElement('div'); var child = document.createElement('span'); \
        parent.appendChild(child); heldList = parent.children; heldList.length; \
        removedWeak = new WeakRef(child); child.remove(); \
    })();",
    )
    .unwrap();
    collect(&mut rt);
    assert!(eval_bool(
        &mut rt,
        "heldList.length === 0 && removedWeak.deref() === undefined"
    ));
}

#[test]
fn cloned_handlers_survive_collection_during_subtree_initialization() {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            super::super::dispatch::COLLECT_CONTENT_HANDLERS.set(false);
        }
    }
    let mut rt = runtime();
    rt.evaluate("var source = document.createElement('div'); source.setAttribute('onclick', 'return 1'); var child = document.createElement('span'); child.setAttribute('onclick', 'return 2'); source.appendChild(child);").unwrap();
    super::super::dispatch::COLLECT_CONTENT_HANDLERS.set(true);
    let _reset = Reset;
    assert!(eval_bool(
        &mut rt,
        "var copy = source.cloneNode(true); copy.onclick() === 1 && copy.firstChild.onclick() === 2"
    ));
}

#[test]
fn current_script_keeps_identity_while_executing_after_removal() {
    let (mut host, _, _, body) = StubHost::page();
    let script = host
        .document
        .append_element(Some(body), "script", None::<&str>);
    host.document.append_text(script, "var runningWeak; (function () { var script = document.currentScript; script.payload = {value: 74}; runningWeak = new WeakRef(script); script.remove(); })(); collectNow(); var runningPreserved = document.currentScript === runningWeak.deref() && document.currentScript.payload.value === 74;");
    let mut rt = DomRuntime::new(host).unwrap();
    rt.context_mut()
        .register_global_builtin_callable(
            boa_engine::JsString::from("collectNow"),
            0,
            boa_engine::NativeFunction::from_fn_ptr(|_, _, context| {
                context.clear_kept_objects();
                boa_gc::force_collect();
                Ok(boa_engine::JsValue::undefined())
            }),
        )
        .unwrap();
    let report = rt.run_document();
    assert!(report.uncaught_errors.is_empty());
    assert!(eval_bool(&mut rt, "runningPreserved"));
    collect(&mut rt);
    assert!(eval_bool(&mut rt, "runningWeak.deref() === undefined"));
}

#[test]
fn written_handlers_survive_collection_before_fragment_attachment() {
    use crate::runtime::host::{BoxGeometry, DocumentHost, HostError};
    use raikiri_dom::Document;
    struct FragmentHost(StubHost);
    impl DocumentHost for FragmentHost {
        fn document(&self) -> &Document {
            &self.0.document
        }
        fn document_mut(&mut self) -> &mut Document {
            &mut self.0.document
        }
        fn flush(&mut self) -> Result<(), HostError> {
            Ok(())
        }
        fn box_geometry(&mut self, _: usize) -> Result<Option<BoxGeometry>, HostError> {
            Ok(None)
        }
        fn computed_value(&mut self, _: usize, _: &str) -> Result<Option<String>, HostError> {
            Ok(None)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
            self
        }
        fn parse_fragment(&mut self, _: &str, _: &str, _: &str) -> Result<Document, HostError> {
            let mut doc = Document::new();
            let root = doc.root_index();
            for (id, code) in [("written-a", "return 10"), ("written-b", "return 20")] {
                let element = doc.append_element(Some(root), "div", None::<&str>);
                doc.set_element_attribute(element, "id", id).unwrap();
                doc.set_element_attribute(element, "onclick", code).unwrap();
            }
            Ok(doc)
        }
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            super::super::dispatch::COLLECT_CONTENT_HANDLERS.set(false);
        }
    }
    let (mut host, _, _, body) = StubHost::page();
    let script = host
        .document
        .append_element(Some(body), "script", None::<&str>);
    host.document
        .append_text(script, "document.write('fixture');");
    let mut rt = DomRuntime::new(FragmentHost(host)).unwrap();
    super::super::dispatch::COLLECT_CONTENT_HANDLERS.set(true);
    let _reset = Reset;
    let report = rt.run_document();
    assert!(report.uncaught_errors.is_empty());
    assert!(eval_bool(
        &mut rt,
        "document.getElementById('written-a').onclick() === 10 && document.getElementById('written-b').onclick() === 20"
    ));
}

#[test]
fn native_bulk_moves_to_new_parents_preserve_wrapper_identity() {
    let (mut host, _, _, body) = StubHost::page();
    let children: Vec<_> = (0..128)
        .map(|_| {
            host.document
                .append_element(Some(body), "div", None::<&str>)
        })
        .collect();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var heldNodes = Array.from(document.body.children); heldNodes.forEach(function (node, index) { node.payload = index; });").unwrap();
    with_state(rt.context_mut(), |state| {
        let doc = state.host.document_mut();
        for child in children {
            let parent = doc.append_element(Some(body), "section", None::<&str>);
            doc.pre_insert(parent, child, None).unwrap();
        }
    })
    .unwrap();
    collect(&mut rt);
    assert!(eval_bool(
        &mut rt,
        "heldNodes.every(function (node, index) { return node.payload === index && node.parentNode.tagName === 'SECTION' && node.parentNode.firstChild === node; })"
    ));
    let debug = with_state(rt.context_mut(), |state| {
        format!("{:?}", state.node_record(body))
    })
    .unwrap();
    assert!(debug.starts_with("NodeRecord { index:"));
}

#[test]
fn detached_css_rule_identity_survives_repeated_cache_sweeps() {
    let mut rt = runtime();
    rt.evaluate("var detachedRules = []; var detachedWeaks = []; var detachedNodeWeak; var detachedSheet; (function () { \
        var node = document.createElement('style'); node.textContent = Array.from({length: 192}, function (_, i) { return '.n' + i + '{color:red}'; }).join(''); \
        document.body.appendChild(node); detachedNodeWeak = new WeakRef(node); detachedSheet = node.sheet; \
        for (var i = 0; i < 192; i++) { var rule = detachedSheet.cssRules[i]; rule.payload = i; detachedRules.push(rule); detachedWeaks.push(new WeakRef(rule)); } \
        for (var i = 0; i < 96; i++) detachedSheet.deleteRule(0); node.remove(); \
    })();").unwrap();
    collect(&mut rt);
    rt.evaluate("for (var i = 0; i < 96; i++) detachedSheet.deleteRule(0); detachedSheet = null;")
        .unwrap();
    collect(&mut rt);
    assert!(eval_bool(
        &mut rt,
        "detachedNodeWeak.deref() === undefined && detachedRules.every(function (rule, index) { return rule === detachedWeaks[index].deref() && rule.payload === index && rule.style.color === 'red'; })"
    ));
    rt.evaluate("detachedRules = null;").unwrap();
    collect(&mut rt);
    assert!(eval_bool(
        &mut rt,
        "detachedWeaks.every(function (ref) { return ref.deref() === undefined; })"
    ));
}

#[test]
fn ownership_initialization_bounds_work_for_cyclic_native_host_edges() {
    let mut document = raikiri_dom::Document::new();
    let root = document.root_index();
    let child = document.append_element(Some(root), "div", None::<&str>);
    // Parser-level attachment trusts its caller. A malformed host must not
    // make ownership initialization loop forever when it supplies a cycle.
    document.attach_child(child, root);
    let graph = crate::runtime::node_gc::NodeGraph::new(&mut document);
    assert_eq!(graph.edge_visits, document.node_count());
}

#[test]
fn replacing_a_connected_sheet_releases_obsolete_rule_payloads_without_a_cssom_read() {
    for mutation in [
        "sheetNode.textContent = 'p {color: blue}'",
        "sheetNode.firstChild.data = 'p {color: blue}'",
        "sheetNode.setAttribute('type', 'text/plain')",
    ] {
        let mut rt = runtime();
        rt.evaluate("var sheetNode = document.createElement('style'); sheetNode.textContent = 'p {color: red}'; document.body.appendChild(sheetNode); var obsoleteWeak; var obsoletePayloadWeak; (function () { var rule = sheetNode.sheet.cssRules[0]; rule.payload = new Uint8Array(1048576); obsoleteWeak = new WeakRef(rule); obsoletePayloadWeak = new WeakRef(rule.payload); })();").unwrap();
        rt.evaluate(mutation).unwrap();
        collect(&mut rt);
        assert!(
            eval_bool(
                &mut rt,
                "obsoleteWeak.deref() === undefined && obsoletePayloadWeak.deref() === undefined"
            ),
            "{mutation}"
        );
    }
}

#[test]
fn css_detachment_keeps_batch_roots_during_collections_between_allocations() {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            crate::runtime::stylesheet::COLLECT_CSS_ALLOCATIONS.set(false);
        }
    }
    for mutation in [
        "batchSheetNode.textContent = 'p {color: blue}'",
        "batchSheetNode.sheet.deleteRule(0)",
    ] {
        let mut rt = runtime();
        rt.evaluate("var batchSheetNode = document.createElement('style'); batchSheetNode.textContent = Array.from({length:16}, function (_, i) { return '@page p' + i + ' { @top-left {content: \"a\"} @top-center {content: \"b\"} @top-right {content: \"c\"} }'; }).join(''); document.body.appendChild(batchSheetNode); (function () { var list = batchSheetNode.sheet.cssRules; for (var i = 0; i < list.length; i++) { var page = list[i]; for (var j = 0; j < page.cssRules.length; j++) page.cssRules[j].payload = j; } })();").unwrap();
        rt.context_mut().clear_kept_objects();
        crate::runtime::stylesheet::COLLECT_CSS_ALLOCATIONS.set(true);
        let _reset = Reset;
        rt.evaluate(mutation).unwrap();
        assert!(eval_bool(
            &mut rt,
            "batchSheetNode.sheet.cssRules.length > 0"
        ));
    }
}
