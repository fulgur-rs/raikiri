use super::*;

#[test]
fn replacement_keeps_fragment_children_and_detaches_old_handles() {
    let mut rt = rt();
    rt.evaluate(
        "var b = document.body; var a = document.createElement('a'); \
         var old = document.createTextNode('old'); var c = document.createElement('c'); \
         b.append(a, old, c); var f = document.createDocumentFragment(); \
         f.append(c, a); b.replaceChildren(f);",
    )
    .unwrap();
    ok(
        &mut rt,
        "b.childNodes.length === 2 && b.firstChild === c && b.lastChild === a \
        && f.childNodes.length === 0 && old.parentNode === null && old.data === 'old'",
    );
    rt.evaluate("b.replaceChildren(a);").unwrap();
    ok(
        &mut rt,
        "b.childNodes.length === 1 && b.firstChild === a && c.parentNode === null",
    );
    rt.evaluate("b.replaceChildren();").unwrap();
    ok(
        &mut rt,
        "b.childNodes.length === 0 && a.parentNode === null && old.data === 'old'",
    );
}

#[test]
fn invalid_replacement_does_not_remove_existing_children() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; var a = b.appendChild(document.createElement('a'));")
        .unwrap();
    ok(
        &mut rt,
        "try { b.replaceChildren(document); false } catch (e) { e.name === 'HierarchyRequestError' }",
    );
    ok(
        &mut rt,
        "b.firstChild === a && b.childNodes.length === 1 && a.parentNode === b",
    );
    ok(
        &mut rt,
        "try { document.replaceChildren(document.createTextNode('bad')); false } catch (e) { e.name === 'HierarchyRequestError' }",
    );
    ok(
        &mut rt,
        "document.documentElement.parentNode === document && a.parentNode === b",
    );
}

#[test]
fn replacement_clears_focus_without_restoring_it_on_reattach() {
    let mut rt = rt();
    rt.evaluate("var b = document.body; var a = b.appendChild(document.createElement('button')); a.focus();").unwrap();
    ok(&mut rt, "document.activeElement === a");
    rt.evaluate("b.replaceChildren(); b.appendChild(a);")
        .unwrap();
    ok(
        &mut rt,
        "document.activeElement !== a && !a.matches(':focus')",
    );
}

#[test]
fn template_fragment_replacement_preserves_template_and_retained_child() {
    let (mut host, _, _, body) = StubHost::page();
    let template = host
        .document
        .append_element(Some(body), "template", None::<&str>);
    let contents = host.document.allocate_template_fragment_root(template);
    let first = host
        .document
        .append_element(Some(contents), "a", None::<&str>);
    let retained = host
        .document
        .append_element(Some(contents), "b", None::<&str>);
    let mut rt = DomRuntime::new(host).unwrap();
    for (name, node) in [
        ("t", template),
        ("contents", contents),
        ("a", first),
        ("b", retained),
    ] {
        expose(&mut rt, name, node);
    }
    rt.evaluate("contents.replaceChildren(b);").unwrap();
    ok(
        &mut rt,
        "t.childNodes.length === 0 && contents.childNodes.length === 1 \
        && contents.firstChild === b && b.parentNode === contents && a.parentNode === null",
    );
}

#[test]
#[ignore = "manual release-mode scaling measurement; no wall-time assertion"]
fn replace_children_scaling_probe() {
    use std::time::Instant;
    for count in [4000, 8000, 16000, 32000, 64000] {
        for replace in [true, false] {
            for sample in 0..6 {
                let (mut host, _, _, body) = StubHost::page();
                let children: Vec<_> = (0..count)
                    .map(|_| {
                        host.document
                            .append_element(Some(body), "div", None::<&str>)
                    })
                    .collect();
                let mut rt = DomRuntime::new(host).unwrap();
                rt.evaluate("var target = document.body;").unwrap();
                let start = Instant::now();
                rt.evaluate(if replace {
                    "target.replaceChildren()"
                } else {
                    "target.textContent = ''"
                })
                .unwrap();
                let elapsed = start.elapsed();
                ok(&mut rt, "target.childNodes.length === 0");
                with_state(rt.context_mut(), |state| {
                    let doc = state.host.document();
                    assert!(doc.get_node(body).unwrap().children.is_empty());
                    assert!(
                        children
                            .iter()
                            .all(|&id| doc.parent_of(id).is_none() && doc.get_node(id).is_some())
                    );
                })
                .unwrap();
                if sample > 0 {
                    println!(
                        "replace_children,{count},{replace},{sample},{}",
                        elapsed.as_nanos()
                    );
                }
            }
        }
    }
}
