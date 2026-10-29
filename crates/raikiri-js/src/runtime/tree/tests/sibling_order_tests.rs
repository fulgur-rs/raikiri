use super::*;

#[test]
#[ignore = "manual release-mode sibling navigation measurement"]
fn sibling_navigation_scaling_probe() {
    use std::time::Instant;
    for mode in ["forward", "backward", "gap_forward", "gap_backward"] {
        for n in [2_000, 4_000, 8_000] {
            let mut samples = Vec::new();
            for _ in 0..5 {
                let (mut host, ..) = StubHost::page();
                let mut document = raikiri_dom::Document::new();
                let html = document.append_element(Some(0), "html", None::<&str>);
                let body = document.append_element(Some(html), "body", None::<&str>);
                for i in 0..n {
                    if mode.starts_with("gap_") && i > 0 && i + 1 < n {
                        document.append_comment(Some(body), "x");
                    } else {
                        let node = document.append_element(Some(body), "div", None::<&str>);
                        document
                            .set_element_attribute(node, "id", format!("id{i}"))
                            .unwrap();
                    }
                }
                document.mark_in_document_flags();
                host.document = document;
                let mut runtime = DomRuntime::new(host).unwrap();
                runtime.evaluate("var b=document.body; var a=b.children; var warm=a.length; var firstExpected=b.firstChild; var lastExpected=b.lastChild; for(var j=0;j<a.length;j++)a[j];").unwrap();
                let source = match mode {
                    "forward" => format!(
                        "(() => {{ let count=0; for(let p=b.firstChild;p;p=p.nextSibling)++count; return count==={n}; }})()"
                    ),
                    "backward" => format!(
                        "(() => {{ let count=0; for(let p=b.lastChild;p;p=p.previousSibling)++count; return count==={n}; }})()"
                    ),
                    "gap_forward" => "b.firstChild.nextElementSibling===b.lastChild".to_owned(),
                    _ => "b.lastChild.previousElementSibling===b.firstChild".to_owned(),
                };
                let start = Instant::now();
                let result = runtime.evaluate(&source).unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1_000.0);
                assert!(result.to_boolean());
                drop(runtime);
                boa_gc::force_collect();
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "siblings mode={mode} n={n} median_ms={:.6} samples={samples:?}",
                samples[2]
            );
        }
    }
}

fn reset_work() {
    SIBLING_WORK.with(|work| work.set([0; 3]));
}

fn work() -> [usize; 3] {
    SIBLING_WORK.with(std::cell::Cell::get)
}

#[test]
fn sibling_chains_have_linear_position_work_in_both_directions() {
    for n in [32, 64, 128] {
        for reverse in [false, true] {
            let (mut host, _, _, body) = StubHost::page();
            for _ in 0..n {
                host.document
                    .append_element(Some(body), "div", None::<&str>);
            }
            let mut runtime = DomRuntime::new(host).unwrap();
            runtime.evaluate("var parent1006=document.body;").unwrap();
            reset_work();
            let (start, step) = if reverse {
                ("lastChild", "previousSibling")
            } else {
                ("firstChild", "nextSibling")
            };
            ok(
                &mut runtime,
                &format!(
                    "(() => {{let count=0; for(let node=parent1006.{start};node;node=node.{step})++count;return count==={n};}})()"
                ),
            );
            assert!(
                work().iter().sum::<usize>() <= 2 * n + 1,
                "n={n}, reverse={reverse}, work={:?}",
                work()
            );
        }
    }
}

#[test]
fn a_single_element_gap_getter_has_linear_work() {
    for n in [32, 64, 128] {
        for reverse in [false, true] {
            let (mut host, _, _, body) = StubHost::page();
            host.document
                .append_element(Some(body), "div", None::<&str>);
            for i in 0..n - 2 {
                match i % 3 {
                    0 => {
                        host.document.append_comment(Some(body), "comment");
                    }
                    1 => {
                        host.document.append_text(body, "text");
                    }
                    _ => {
                        host.document
                            .append_processing_instruction(Some(body), "target", "data");
                    }
                }
            }
            host.document
                .append_element(Some(body), "div", None::<&str>);
            let mut runtime = DomRuntime::new(host).unwrap();
            reset_work();
            let script = if reverse {
                "document.body.lastChild.previousElementSibling===document.body.firstChild"
            } else {
                "document.body.firstChild.nextElementSibling===document.body.lastChild"
            };
            ok(&mut runtime, script);
            assert!(
                work().iter().sum::<usize>() <= 2 * n,
                "n={n}, reverse={reverse}, work={:?}",
                work()
            );
        }
    }
}

#[test]
fn warmed_positions_follow_before_argument_moves_and_split_text_insertion() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "var group1006=document.createElement('section'); document.body.append(group1006); var items1006=['a','b','c','d','e'].map(name=>document.createElement(name)); group1006.append(...items1006); items1006[2].nextSibling===items1006[3] && items1006[2].previousSibling===items1006[1]",
    );
    ok(
        &mut runtime,
        "items1006[3].before(items1006[0],items1006[4]); group1006.childNodes[0]===items1006[1] && group1006.childNodes[1]===items1006[2] && group1006.childNodes[2]===items1006[0] && group1006.childNodes[3]===items1006[4] && group1006.childNodes[4]===items1006[3]",
    );
    ok(
        &mut runtime,
        "var text1006=document.createTextNode('abcd'); var after1006=document.createElement('i'); group1006.replaceChildren(text1006,after1006); text1006.nextSibling===after1006 && after1006.previousSibling===text1006",
    );
    ok(
        &mut runtime,
        "var tail1006=text1006.splitText(2); text1006.data==='ab' && tail1006.data==='cd' && text1006.nextSibling===tail1006 && tail1006.nextSibling===after1006 && after1006.previousSibling===tail1006",
    );
}

#[test]
fn detached_fragments_and_template_contents_keep_source_sibling_order() {
    let (mut host, _, _, body) = StubHost::page();
    let template = host
        .document
        .append_element(Some(body), "template", None::<&str>);
    let contents = host.document.allocate_template_fragment_root(template);
    let first = host
        .document
        .append_element(Some(contents), "a", None::<&str>);
    let text = host.document.append_text(contents, "text");
    let comment = host.document.append_comment(Some(contents), "comment");
    let pi = host
        .document
        .append_processing_instruction(Some(contents), "target", "data");
    let last = host
        .document
        .append_element(Some(contents), "b", None::<&str>);
    let mut runtime = DomRuntime::new(host).unwrap();
    for (name, node) in [
        ("template1006", template),
        ("contents1006", contents),
        ("first1006", first),
        ("text1006", text),
        ("comment1006", comment),
        ("pi1006", pi),
        ("last1006", last),
    ] {
        expose(&mut runtime, name, node);
    }
    ok(
        &mut runtime,
        "first1006.nextSibling===text1006 && text1006.nextSibling===comment1006 && comment1006.nextSibling===pi1006 && pi1006.nextSibling===last1006 && last1006.previousSibling===pi1006 && text1006.previousSibling===first1006",
    );
    ok(
        &mut runtime,
        "first1006.nextElementSibling===last1006 && last1006.previousElementSibling===first1006 && text1006.nextElementSibling===last1006 && pi1006.previousElementSibling===first1006",
    );
    ok(
        &mut runtime,
        "document.body.appendChild(first1006); first1006.previousSibling===template1006 && first1006.nextSibling===null && text1006.previousSibling===null && last1006.previousElementSibling===null",
    );
    ok(
        &mut runtime,
        "contents1006.insertBefore(first1006,last1006); first1006.previousSibling===pi1006 && first1006.nextSibling===last1006 && text1006.nextElementSibling===first1006 && last1006.previousElementSibling===first1006",
    );
    ok(
        &mut runtime,
        "document.body.appendChild(contents1006); contents1006.firstChild===null && template1006.nextSibling===text1006 && text1006.previousSibling===template1006 && first1006.nextSibling===last1006",
    );
}

#[test]
fn sibling_position_cache_does_not_root_detached_wrappers_or_payloads() {
    let mut runtime = rt();
    runtime.evaluate("var siblingWeak1006; var payloadWeak1006; (()=>{ let first=document.createElement('a'); let second=document.createElement('b'); first.payload=new Uint8Array(1048576); document.body.append(first,second); if(first.nextSibling!==second || first.nextSibling!==second)throw new Error('siblings'); siblingWeak1006=new WeakRef(first); payloadWeak1006=new WeakRef(first.payload); first.remove(); second.remove(); })()").unwrap();
    runtime.context_mut().clear_kept_objects();
    boa_gc::force_collect();
    ok(
        &mut runtime,
        "siblingWeak1006.deref()===undefined && payloadWeak1006.deref()===undefined",
    );
}
