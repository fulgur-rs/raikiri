use super::*;

#[test]
fn tree_walker_has_linear_sibling_work_for_flat_skipped_and_deep_trees() {
    for n in [32, 64, 128] {
        for shape in ["flat", "skipped", "deep"] {
            for show in [0, 4] {
                let (mut host, _, _, body) = StubHost::page();
                let mut parent = body;
                for _ in 0..n {
                    if shape == "deep" {
                        parent = host
                            .document
                            .append_element(Some(parent), "div", None::<&str>);
                    } else if shape == "skipped" {
                        host.document.append_comment(Some(parent), "ignored");
                        let child =
                            host.document
                                .append_element(Some(parent), "span", None::<&str>);
                        host.document.append_text(child, "nested");
                    }
                    host.document.append_text(parent, "X");
                }
                let expected = if show == 0 {
                    0
                } else if shape == "skipped" {
                    n * 2
                } else {
                    n
                };
                let mut runtime = DomRuntime::new(host).unwrap();
                SIBLING_WORK.with(|work| work.set([0; 3]));
                ok(
                    &mut runtime,
                    &format!(
                        "var w=document.createTreeWalker(document.body,{show}); var visited=0; while(w.nextNode()!==null)visited++; visited==={expected} && w.nextNode()===null"
                    ),
                );
                let work = SIBLING_WORK.with(std::cell::Cell::get);
                let entries: usize = work.iter().sum();
                eprintln!("treewalker shape={shape} show={show} n={n} work={work:?}");
                assert!(
                    entries <= 12 * n,
                    "shape={shape} show={show} n={n} work={work:?}"
                );
            }
        }
    }
}

#[test]
fn tree_walker_observes_mutations_after_warming_shared_sibling_positions() {
    let mut runtime = rt();
    ok(
        &mut runtime,
        "var p=document.createElement('section'); document.body.append(p); var a=document.createTextNode('a'), b=document.createTextNode('b'), c=document.createTextNode('c'); p.append(a,b,c); var w=document.createTreeWalker(p,4), other=document.createTreeWalker(p,4); w.nextNode()===a && w.nextNode()===b && w.nextNode()===c && w.nextNode()===null && w.currentNode===c",
    );
    ok(
        &mut runtime,
        "w.currentNode=a; var inserted=document.createTextNode('i'); p.insertBefore(inserted,b); b.remove(); w.nextNode()===inserted && w.nextNode()===c && w.nextNode()===null && other.nextNode()===a",
    );
    ok(
        &mut runtime,
        "p.insertBefore(c,a); w.currentNode=p; w.nextNode()===c && w.nextNode()===a && w.nextNode()===inserted && w.nextNode()===null",
    );
    ok(
        &mut runtime,
        "w.currentNode=a; var tail=a.splitText(1); w.nextNode()===tail && w.nextNode()===inserted",
    );
    ok(
        &mut runtime,
        "var detached=document.createDocumentFragment(); detached.append(a,tail); w.currentNode=detached; w.nextNode()===a && w.nextNode()===tail && w.nextNode()===null && w.currentNode===tail",
    );
    ok(
        &mut runtime,
        "w.currentNode=p; w.nextNode()===c && w.nextNode()===inserted && w.nextNode()===null && w.currentNode===inserted; var none=document.createTreeWalker(p,0); none.nextNode()===null && none.currentNode===p",
    );
}

#[test]
fn tree_walker_observes_native_mutation_without_runtime_dirty_notification() {
    let (mut host, _, _, body) = StubHost::page();
    let a = host.document.append_text(body, "a");
    let b = host.document.append_text(body, "b");
    let c = host.document.append_text(body, "c");
    let mut runtime = DomRuntime::new(host).unwrap();
    expose(&mut runtime, "a", a);
    expose(&mut runtime, "b", b);
    expose(&mut runtime, "c", c);
    ok(
        &mut runtime,
        "var w=document.createTreeWalker(document.body,4); w.nextNode()===a && w.nextNode()===b && w.nextNode()===c",
    );
    with_state(runtime.context_mut(), |state| {
        state
            .host
            .document_mut()
            .pre_insert(body, c, Some(a))
            .unwrap();
    })
    .unwrap();
    ok(
        &mut runtime,
        "w.currentNode=c; w.nextNode()===a && w.nextNode()===b && w.nextNode()===null && w.currentNode===b",
    );
    with_state(runtime.context_mut(), |state| {
        state.host.document_mut().detach_from_parent(a);
    })
    .unwrap();
    ok(
        &mut runtime,
        "w.currentNode=c; w.nextNode()===b && w.nextNode()===null; w.currentNode=a; w.nextNode()===null && w.currentNode===a",
    );
}

#[test]
fn tree_walker_refreshes_positions_when_native_document_identity_changes() {
    fn document(reverse: bool) -> (raikiri_dom::Document, [usize; 3]) {
        let mut doc = raikiri_dom::Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let nodes = [
            doc.create_detached_text("a"),
            doc.create_detached_text("b"),
            doc.create_detached_text("c"),
        ];
        let [a, b, c] = nodes;
        for node in if reverse { [c, a, b] } else { [a, b, c] } {
            doc.attach_child(body, node);
        }
        (doc, nodes)
    }
    let (mut host, ..) = StubHost::page();
    let (first, [a, b, c]) = document(false);
    host.document = first;
    let mut runtime = DomRuntime::new(host).unwrap();
    expose(&mut runtime, "a", a);
    expose(&mut runtime, "b", b);
    expose(&mut runtime, "c", c);
    ok(
        &mut runtime,
        "var w=document.createTreeWalker(document.body,4); w.nextNode()===a && w.nextNode()===b && w.nextNode()===c",
    );
    let (replacement, nodes) = document(true);
    assert_eq!(nodes, [a, b, c]);
    with_state(runtime.context_mut(), |state| {
        state
            .host
            .as_any_mut()
            .downcast_mut::<StubHost>()
            .unwrap()
            .document = replacement;
    })
    .unwrap();
    ok(
        &mut runtime,
        "w.currentNode=c; w.nextNode()===a && w.nextNode()===b && w.nextNode()===null && w.currentNode===b",
    );
}
