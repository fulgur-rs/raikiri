use super::*;

#[test]
fn repeated_missing_and_tail_id_queries_walk_only_once() {
    for n in [32, 64, 128] {
        for missing in [false, true] {
            let (mut host, _, _, body) = StubHost::page();
            for i in 0..n {
                let child = host
                    .document
                    .append_element(Some(body), "div", None::<&str>);
                host.document
                    .set_element_attribute(child, "id", format!("id{i}"))
                    .unwrap();
            }
            let mut runtime = DomRuntime::new(host).unwrap();
            ID_VISITS.with(|v| v.set(0));
            let id = if missing {
                "missing".to_owned()
            } else {
                format!("id{}", n - 1)
            };
            ok(
                &mut runtime,
                &format!(
                    "(()=>{{let value;for(let i=0;i<{n};++i)value=document.getElementById('{id}');return {};}})()",
                    if missing {
                        "value===null"
                    } else {
                        "value===document.body.lastChild"
                    }
                ),
            );
            assert_eq!(
                ID_VISITS.with(std::cell::Cell::get),
                n + 3,
                "n={n}, missing={missing}"
            );
        }
    }
}

#[test]
fn cached_ids_follow_duplicate_order_attributes_and_fragment_scopes() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "var a1008=document.createElement('a'); var b1008=document.createElement('b'); a1008.id=b1008.id='dup1008'; document.body.append(a1008,b1008); document.getElementById('dup1008')===a1008 && document.getElementById('dup1008')===a1008 && document.getElementById('')===null",
    );
    ok(
        &mut runtime,
        "document.body.prepend(b1008); document.getElementById('dup1008')===b1008",
    );
    ok(
        &mut runtime,
        "b1008.removeAttribute('id'); document.getElementById('dup1008')===a1008",
    );
    ok(
        &mut runtime,
        "a1008.id='new1008'; document.getElementById('dup1008')===null && document.getElementById('new1008')===a1008",
    );
    ok(
        &mut runtime,
        "a1008.remove(); document.getElementById('new1008')===null",
    );
    ok(
        &mut runtime,
        "var f1008=document.createDocumentFragment();var g1008=document.createDocumentFragment(); var c1008=document.createElement('c');c1008.id='new1008';f1008.append(a1008);g1008.append(c1008);f1008.getElementById('new1008')===a1008 && g1008.getElementById('new1008')===c1008 && document.getElementById('new1008')===null",
    );
    ok(
        &mut runtime,
        "g1008.prepend(a1008);f1008.getElementById('new1008')===null && g1008.getElementById('new1008')===a1008",
    );
    ok(
        &mut runtime,
        "document.body.append(g1008);g1008.getElementById('new1008')===null && document.getElementById('new1008')===a1008",
    );
    ok(
        &mut runtime,
        "var named1008=document.createElement('input');named1008.setAttribute('name','onlyName1008');document.body.append(named1008);document.getElementById('onlyName1008')===null",
    );
}

#[test]
fn argument_coercion_mutation_precedes_cache_version_validation() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "var coerce1008=document.createElement('a');coerce1008.id='coerce1008';document.body.append(coerce1008);document.getElementById('coerce1008')===coerce1008 && document.getElementById('after1008')===null",
    );
    ok(
        &mut runtime,
        "document.getElementById({toString(){coerce1008.id='after1008';return 'coerce1008';}})===null && document.getElementById('after1008')===coerce1008",
    );
    ok(
        &mut runtime,
        "document.getElementById({toString(){coerce1008.remove();return 'after1008';}})===null",
    );
    ok(
        &mut runtime,
        "document.getElementById({toString(){document.body.append(coerce1008);return '';}})===null && document.getElementById('after1008')===coerce1008",
    );
}

#[test]
fn native_id_and_topology_changes_refresh_without_js_generation() {
    let (mut host, _, _, body) = StubHost::page();
    let template = host
        .document
        .append_element(Some(body), "template", None::<&str>);
    let contents = host.document.allocate_template_fragment_root(template);
    let inert = host
        .document
        .append_element(Some(contents), "a", None::<&str>);
    host.document
        .set_element_attribute(inert, "id", "native1008")
        .unwrap();
    let svg = host
        .document
        .append_element(Some(body), "svg", None::<&str>);
    host.document
        .set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    host.document
        .set_element_attribute(svg, "id", "svg1008")
        .unwrap();
    let mut runtime = DomRuntime::new(host).unwrap();
    expose(&mut runtime, "contents1008", contents);
    expose(&mut runtime, "inert1008", inert);
    expose(&mut runtime, "svg1008", svg);
    expose(&mut runtime, "template1008", template);
    ok(
        &mut runtime,
        "document.getElementById('native1008')===null && contents1008.getElementById('native1008')===inert1008 && document.getElementById('svg1008')===svg1008",
    );
    let generation = with_state(runtime.context_mut(), |s| s.generation).unwrap();
    with_state(runtime.context_mut(), |s| {
        let doc = s.host.document_mut();
        doc.pre_insert(template, inert, None).unwrap();
        doc.set_element_attribute(svg, "id", "renamed1008").unwrap();
    })
    .unwrap();
    ok(
        &mut runtime,
        "document.getElementById('native1008')===inert1008 && contents1008.getElementById('native1008')===null && document.getElementById('svg1008')===null && document.getElementById('renamed1008')===svg1008",
    );
    with_state(runtime.context_mut(), |s| {
        s.host.document_mut().detach_from_parent(inert)
    })
    .unwrap();
    ok(&mut runtime, "document.getElementById('native1008')===null");
    assert_eq!(
        with_state(runtime.context_mut(), |s| s.generation).unwrap(),
        generation
    );
}

#[test]
fn cached_id_results_do_not_root_detached_wrappers_or_payloads() {
    let mut runtime = rt();
    runtime.evaluate("var weak1008;var payload1008;(()=>{let node=document.createElement('a');node.id='collect1008';node.payload=new Uint8Array(1048576);document.body.append(node);if(document.getElementById('collect1008')!==node || document.getElementById('collect1008')!==node)throw new Error('id');weak1008=new WeakRef(node);payload1008=new WeakRef(node.payload);node.remove();})()").unwrap();
    runtime.context_mut().clear_kept_objects();
    boa_gc::force_collect();
    ok(
        &mut runtime,
        "weak1008.deref()===undefined && payload1008.deref()===undefined",
    );
}
