use super::*;

fn promote_root(runtime: &mut DomRuntime, root: &str) {
    ok(
        runtime,
        &format!(
            "(()=>{{const root={root};for(let i=0;i<=128;i++)if(root.getElementById('__promote1143'+i)!==null)throw new Error('fixture');return true;}})()"
        ),
    );
    ID_VISITS.with(|value| value.set(0));
    ok(
        runtime,
        &format!("{root}.getElementById('__promoted_new_missing1143')===null"),
    );
    assert_eq!(ID_VISITS.with(std::cell::Cell::get), 0);
}

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
    promote_root(&mut runtime, "document");
    ok(
        &mut runtime,
        "document.body.prepend(b1008); document.getElementById('dup1008')===b1008",
    );
    promote_root(&mut runtime, "document");
    ok(
        &mut runtime,
        "b1008.removeAttribute('id'); document.getElementById('dup1008')===a1008",
    );
    promote_root(&mut runtime, "document");
    ok(
        &mut runtime,
        "a1008.id='new1008'; document.getElementById('dup1008')===null && document.getElementById('new1008')===a1008",
    );
    promote_root(&mut runtime, "document");
    ok(
        &mut runtime,
        "a1008.remove(); document.getElementById('new1008')===null",
    );
    ok(
        &mut runtime,
        "var f1008=document.createDocumentFragment();var g1008=document.createDocumentFragment(); var c1008=document.createElement('c');c1008.id='new1008';f1008.append(a1008);g1008.append(c1008);f1008.getElementById('new1008')===a1008 && g1008.getElementById('new1008')===c1008 && document.getElementById('new1008')===null",
    );
    promote_root(&mut runtime, "f1008");
    promote_root(&mut runtime, "g1008");
    ok(
        &mut runtime,
        "g1008.prepend(a1008);f1008.getElementById('new1008')===null && g1008.getElementById('new1008')===a1008",
    );
    promote_root(&mut runtime, "f1008");
    promote_root(&mut runtime, "g1008");
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
    promote_root(&mut runtime, "document");
    ok(
        &mut runtime,
        "document.getElementById({toString(){coerce1008.id='after1008';return 'coerce1008';}})===null && document.getElementById('after1008')===coerce1008",
    );
    promote_root(&mut runtime, "document");
    ok(
        &mut runtime,
        "document.getElementById({toString(){coerce1008.remove();return 'after1008';}})===null",
    );
    promote_root(&mut runtime, "document");
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
    promote_root(&mut runtime, "document");
    promote_root(&mut runtime, "contents1008");
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
    promote_root(&mut runtime, "document");
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
    runtime.evaluate("var weak1008;var payload1008;(()=>{let node=document.createElement('a');node.id='collect1008';node.payload=new Uint8Array(1048576);document.body.append(node);if(document.getElementById('collect1008')!==node || document.getElementById('collect1008')!==node)throw new Error('id');for(let i=0;i<=128;i++)document.getElementById('__gc1143'+i);weak1008=new WeakRef(node);payload1008=new WeakRef(node.payload);node.remove();})()").unwrap();
    runtime.context_mut().clear_kept_objects();
    boa_gc::force_collect();
    ok(
        &mut runtime,
        "weak1008.deref()===undefined && payload1008.deref()===undefined",
    );
}

#[test]
fn stable_existing_id_working_sets_do_not_rescan_after_warmup() {
    for count in [128, 129, 256] {
        let (mut host, _, _, body) = StubHost::page();
        for _ in 0..128 {
            host.document
                .append_element(Some(body), "div", None::<&str>);
        }
        for index in 0..count {
            let node = host
                .document
                .append_element(Some(body), "div", None::<&str>);
            host.document
                .set_element_attribute(node, "id", format!("key{index}"))
                .unwrap();
        }
        let mut runtime = DomRuntime::new(host).unwrap();
        ok(
            &mut runtime,
            &format!(
                "var refs1143=[];for(var i=0;i<{count};i++)refs1143.push(document.getElementById('key'+i));refs1143.every(x=>x!==null)"
            ),
        );
        for mixed in [false, true] {
            ID_VISITS.with(|value| value.set(0));
            let missing = if mixed {
                "if(document.getElementById('missing'+i)!==null)throw new Error('missing');"
            } else {
                ""
            };
            ok(
                &mut runtime,
                &format!(
                    "var good1143=0;for(var r=0;r<3;r++)for(var i=0;i<{count};i++){{if(document.getElementById('key'+i)===refs1143[i])good1143++;{missing}}}good1143==={count}*3"
                ),
            );
            // A128-key scalar working set may promote on the first missing key.
            let expected_bound = if mixed && count == 128 {
                128 + count + 3
            } else {
                0
            };
            let visits = ID_VISITS.with(std::cell::Cell::get);
            assert!(
                visits <= expected_bound,
                "keys={count}, mixed={mixed}, visits={visits}, bound={expected_bound}"
            );
        }
    }
}

#[test]
fn small_head_working_set_keeps_early_stopping_before_capacity_pressure() {
    let (mut host, _, _, body) = StubHost::page();
    for index in 0..2 {
        let node = host
            .document
            .append_element(Some(body), "div", None::<&str>);
        host.document
            .set_element_attribute(node, "id", format!("head{index}"))
            .unwrap();
    }
    for _ in 0..16_000 {
        host.document
            .append_element(Some(body), "div", None::<&str>);
    }
    let mut runtime = DomRuntime::new(host).unwrap();
    ID_VISITS.with(|value| value.set(0));
    ok(
        &mut runtime,
        "var a1143=document.getElementById('head0');var b1143=document.getElementById('head1');for(var i=0;i<100;i++){if(document.getElementById('head0')!==a1143||document.getElementById('head1')!==b1143)throw new Error('head');}a1143!==null&&b1143!==null",
    );
    assert_eq!(ID_VISITS.with(std::cell::Cell::get), 9);
}
