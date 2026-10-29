use super::*;

#[test]
#[ignore = "manual release-mode Element child getter measurement"]
fn element_child_scaling_probe() {
    use std::time::Instant;
    for mode in ["first", "last", "count"] {
        for n in [2_000, 4_000, 8_000] {
            let mut samples = Vec::new();
            for _ in 0..5 {
                let (mut host, ..) = StubHost::page();
                let mut document = raikiri_dom::Document::new();
                let html = document.append_element(Some(0), "html", None::<&str>);
                let body = document.append_element(Some(html), "body", None::<&str>);
                for i in 0..n {
                    let node = document.append_element(Some(body), "div", None::<&str>);
                    document
                        .set_element_attribute(node, "id", format!("id{i}"))
                        .unwrap();
                }
                document.mark_in_document_flags();
                host.document = document;
                let mut runtime = DomRuntime::new(host).unwrap();
                runtime.evaluate("var b=document.body; var a=b.children; var warm=a.length; var firstExpected=b.firstChild; var lastExpected=b.lastChild; for(var j=0;j<a.length;j++)a[j];").unwrap();
                let (property, expected) = match mode {
                    "first" => ("firstElementChild", "firstExpected".to_owned()),
                    "last" => ("lastElementChild", "lastExpected".to_owned()),
                    _ => ("childElementCount", n.to_string()),
                };
                let source = format!(
                    "(() => {{let value;for(let i=0;i<{n};++i)value=b.{property};return value==={expected};}})()"
                );
                let start = Instant::now();
                let result = runtime.evaluate(&source).unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1_000.0);
                assert!(result.to_boolean());
                drop(runtime);
                boa_gc::force_collect();
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "element child mode={mode} n={n} queries={n} median_ms={:.6} samples={samples:?}",
                samples[2]
            );
        }
    }
}

#[test]
fn repeated_element_child_getters_have_linear_work_without_child_vectors() {
    for n in [32, 64, 128] {
        for (kind, property) in ["firstElementChild", "lastElementChild", "childElementCount"]
            .into_iter()
            .enumerate()
        {
            let (mut host, _, _, body) = StubHost::page();
            for _ in 0..n {
                host.document
                    .append_element(Some(body), "div", None::<&str>);
            }
            let mut runtime = DomRuntime::new(host).unwrap();
            runtime.evaluate("var parent1007=document.body;").unwrap();
            ELEMENT_CHILD_WORK.with(|work| work.set([0; 4]));
            runtime
                .evaluate(&format!("for(let i=0;i<{n};++i)parent1007.{property};"))
                .unwrap();
            let work = ELEMENT_CHILD_WORK.with(std::cell::Cell::get);
            assert_eq!(work[kind], n, "property={property}, n={n}, work={work:?}");
            assert_eq!(work[3], 0, "no child ID vector for {property}");
        }
    }
}

#[test]
fn element_child_getters_follow_mixed_children_and_script_mutations() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "var p1007=document.createElement('section'); p1007.firstElementChild===null && p1007.lastElementChild===null && p1007.childElementCount===0",
    );
    ok(
        &mut runtime,
        "var a1007=document.createElement('a'); var b1007=document.createElement('b'); p1007.append('prefix',document.createComment('c'),a1007,'gap',b1007,'suffix'); p1007.firstElementChild===a1007 && p1007.lastElementChild===b1007 && p1007.childElementCount===2 && p1007.childElementCount===2",
    );
    ok(
        &mut runtime,
        "p1007.insertBefore(b1007,a1007); p1007.firstElementChild===b1007 && p1007.lastElementChild===a1007 && p1007.childElementCount===2",
    );
    ok(
        &mut runtime,
        "a1007.remove(); p1007.firstElementChild===b1007 && p1007.lastElementChild===b1007 && p1007.childElementCount===1",
    );
    ok(
        &mut runtime,
        "var f1007=document.createDocumentFragment(); f1007.append(a1007,document.createElement('i')); f1007.childElementCount===2 && f1007.firstElementChild===a1007 && f1007.lastElementChild.tagName==='I'",
    );
    ok(
        &mut runtime,
        "p1007.append(f1007); f1007.childElementCount===0 && f1007.firstElementChild===null && f1007.lastElementChild===null && p1007.childElementCount===3",
    );
    ok(
        &mut runtime,
        "p1007.textContent='only text'; p1007.firstElementChild===null && p1007.lastElementChild===null && p1007.childElementCount===0 && p1007.childElementCount===0",
    );
    ok(
        &mut runtime,
        "p1007.replaceChildren(a1007); p1007.childElementCount===1 && p1007.firstElementChild===a1007 && p1007.lastElementChild===a1007 && document.childElementCount===1 && document.firstElementChild===document.documentElement && document.lastElementChild===document.documentElement",
    );
}

#[test]
fn native_template_fragment_changes_refresh_counts_without_js_generation() {
    let (mut host, _, _, body) = StubHost::page();
    let template = host
        .document
        .append_element(Some(body), "template", None::<&str>);
    let contents = host.document.allocate_template_fragment_root(template);
    host.document
        .append_processing_instruction(Some(contents), "target", "data");
    host.document.append_text(contents, "prefix");
    let first = host
        .document
        .append_element(Some(contents), "a", None::<&str>);
    host.document.append_comment(Some(contents), "suffix");
    let mut runtime = DomRuntime::new(host).unwrap();
    expose(&mut runtime, "contents1007", contents);
    expose(&mut runtime, "first1007", first);
    expose(&mut runtime, "template1007", template);
    ok(
        &mut runtime,
        "contents1007.childElementCount===1 && contents1007.firstElementChild===first1007 && contents1007.lastElementChild===first1007 && template1007.childElementCount===0",
    );
    let generation = with_state(runtime.context_mut(), |s| s.generation).unwrap();
    with_state(runtime.context_mut(), |s| {
        s.host.document_mut().detach_from_parent(first)
    })
    .unwrap();
    ok(
        &mut runtime,
        "contents1007.childElementCount===0 && contents1007.firstElementChild===null && contents1007.lastElementChild===null",
    );
    with_state(runtime.context_mut(), |s| {
        s.host.document_mut().attach_child(contents, first)
    })
    .unwrap();
    ok(
        &mut runtime,
        "contents1007.childElementCount===1 && contents1007.firstElementChild===first1007 && contents1007.lastElementChild===first1007",
    );
    assert_eq!(
        with_state(runtime.context_mut(), |s| s.generation).unwrap(),
        generation
    );
}

#[test]
fn child_count_metadata_does_not_root_detached_payloads() {
    let mut runtime = rt();
    runtime.evaluate("var weak1007; var payload1007; (()=>{let parent=document.createElement('section'); let child=document.createElement('a'); child.payload=new Uint8Array(1048576); parent.append(child); if(parent.childElementCount!==1 || parent.childElementCount!==1)throw new Error('count'); weak1007=new WeakRef(child); payload1007=new WeakRef(child.payload);})()").unwrap();
    runtime.context_mut().clear_kept_objects();
    boa_gc::force_collect();
    ok(
        &mut runtime,
        "weak1007.deref()===undefined && payload1007.deref()===undefined",
    );
}
