use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;

use super::take_list_work;

fn ok(runtime: &mut DomRuntime, source: &str) {
    assert!(runtime.evaluate(source).unwrap().to_boolean(), "{source}");
}

#[test]
fn unchanged_stylesheet_list_reads_do_not_walk_the_dom() {
    let (mut host, _, head, body) = StubHost::page();
    host.document
        .append_element(Some(head), "style", None::<&str>);
    for _ in 0..128 {
        host.document
            .append_element(Some(body), "div", None::<&str>);
    }
    let mut runtime = DomRuntime::new(host).unwrap();
    ok(
        &mut runtime,
        "var sheets=document.styleSheets;var first=sheets[0];sheets.length===1",
    );
    take_list_work();
    ok(
        &mut runtime,
        "(()=>{for(let i=0;i<128;++i)if(sheets.length!==1||sheets.item(0)!==first||sheets[0]!==first||sheets.item(1)!==null)return false;return true;})()",
    );
    assert_eq!(take_list_work(), [0, 0]);
}

#[test]
fn unchanged_style_sheet_getters_share_the_list_index() {
    for size in [100, 500, 2_000, 8_000] {
        let (mut host, _, head, body) = StubHost::page();
        let style = host
            .document
            .append_element(Some(head), "style", None::<&str>);
        host.document
            .set_element_attribute(style, "id", "subject")
            .unwrap();
        for _ in 0..size {
            host.document
                .append_element(Some(body), "div", None::<&str>);
        }
        let mut runtime = DomRuntime::new(host).unwrap();
        ok(
            &mut runtime,
            "var subject=document.getElementById('subject');var sheets=document.styleSheets;var first=sheets.item(0);first!==null",
        );
        take_list_work();
        ok(
            &mut runtime,
            "(()=>{for(let i=0;i<100;++i)if(subject.sheet!==first)return false;return true;})()",
        );
        assert_eq!(take_list_work(), [0, 0], "DOM size {size}");
    }
}

#[test]
fn style_sheet_getters_refresh_before_list_reads_for_html_and_svg() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    for namespace in ["http://www.w3.org/1999/xhtml", "http://www.w3.org/2000/svg"] {
        ok(
            &mut runtime,
            &format!("var el=document.createElementNS('{namespace}','style');el.sheet===null"),
        );
        ok(
            &mut runtime,
            "document.head.appendChild(el);var sheet=el.sheet;sheet!==null && document.styleSheets.item(0)===sheet",
        );
        take_list_work();
        ok(
            &mut runtime,
            "el.sheet===sheet && document.styleSheets.length===1",
        );
        assert_eq!(take_list_work(), [0, 0]);
        ok(
            &mut runtime,
            "el.setAttribute('type','text/plain');el.sheet===null && document.styleSheets.length===0",
        );
        ok(
            &mut runtime,
            "el.setAttribute('type','TEXT/CSS');el.sheet===sheet && document.styleSheets.item(0)===sheet",
        );
        ok(
            &mut runtime,
            "document.body.appendChild(el);el.sheet===sheet && document.styleSheets.item(0)===sheet",
        );
        ok(
            &mut runtime,
            "var fragment=document.createDocumentFragment();fragment.appendChild(el);el.sheet===null && document.styleSheets.length===0",
        );
        ok(
            &mut runtime,
            "document.head.appendChild(fragment);el.sheet===sheet && document.styleSheets.item(0)===sheet",
        );
        ok(
            &mut runtime,
            "el.remove();el.sheet===null && document.styleSheets.length===0",
        );
    }
}

use crate::runtime::webidl::with_state;

#[test]
fn empty_lists_and_native_type_namespace_and_document_changes_refresh_without_js_generation() {
    let (host, _, head, _) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    ok(
        &mut runtime,
        "var sheets=document.styleSheets;sheets.length===0 && sheets.item(0)===null",
    );
    take_list_work();
    ok(&mut runtime, "sheets.length===0 && sheets.item(99)===null");
    assert_eq!(take_list_work(), [0, 0]);
    let generation = with_state(runtime.context_mut(), |s| s.generation).unwrap();
    let style = with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .append_element(Some(head), "style", None::<&str>)
    })
    .unwrap();
    ok(
        &mut runtime,
        "var first=sheets[0];sheets.length===1 && first!==null",
    );
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .set_element_attribute(style, "type", "text/plain")
            .unwrap()
    })
    .unwrap();
    ok(&mut runtime, "sheets.length===0 && sheets.item(0)===null");
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .set_element_attribute(style, "type", "TEXT/CSS")
            .unwrap()
    })
    .unwrap();
    ok(&mut runtime, "sheets.length===1 && sheets[0]===first");
    for namespace in [
        Some("http://www.w3.org/2000/svg"),
        Some("urn:foreign"),
        None,
    ] {
        with_state(runtime.context_mut(), |s| {
            s.host
                .document_mut()
                .set_element_namespace(style, namespace.map(Into::into))
        })
        .unwrap();
        if namespace == Some("urn:foreign") {
            ok(&mut runtime, "sheets.length===0 && sheets.item(0)===null");
        } else {
            ok(&mut runtime, "sheets.length===1 && sheets[0]===first");
        }
    }
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .set_element_attributes(style, vec![("type".into(), "text/plain".into())])
    })
    .unwrap();
    ok(&mut runtime, "sheets.length===0");
    with_state(runtime.context_mut(), |s| {
        s.host.document_mut().set_element_attributes(style, vec![])
    })
    .unwrap();
    ok(&mut runtime, "sheets.length===1 && sheets[0]===first");
    with_state(runtime.context_mut(), |s| {
        let clone = s.host.document().clone();
        *s.host.document_mut() = clone;
    })
    .unwrap();
    take_list_work();
    ok(&mut runtime, "sheets.length===1 && sheets[0]===first");
    assert_eq!(take_list_work()[0], 1);
    assert_eq!(
        with_state(runtime.context_mut(), |s| s.generation).unwrap(),
        generation
    );
}

#[test]
fn stylesheet_order_reparenting_fragments_and_argument_coercion_keep_wrapper_identity() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    ok(
        &mut runtime,
        "var sheets=document.styleSheets;var a=document.createElement('style'),b=document.createElement('style');document.head.append(a,b);var sa=a.sheet,sb=b.sheet;sheets.length===2 && sheets[0]===sa && sheets[1]===sb",
    );
    ok(
        &mut runtime,
        "document.head.insertBefore(b,a);sheets[0]===sb && sheets[1]===sa",
    );
    ok(
        &mut runtime,
        "document.body.appendChild(a);sheets[0]===sb && sheets[1]===sa",
    );
    ok(
        &mut runtime,
        "b.remove();sheets.length===1 && sheets[0]===sa",
    );
    ok(
        &mut runtime,
        "var f=document.createDocumentFragment();f.appendChild(b);document.head.appendChild(f);sheets.length===2 && sheets[0]===sb && sheets[1]===sa",
    );
    ok(
        &mut runtime,
        "sheets.item({valueOf(){b.remove();return 0;}})===sa && sheets.length===1",
    );
}

#[test]
fn template_contents_and_detached_subtrees_enter_the_list_only_when_connected() {
    let (mut host, _, head, _) = StubHost::page();
    let template = host
        .document
        .append_element(Some(head), "template", None::<&str>);
    let contents = host.document.allocate_template_fragment_root(template);
    let hidden = host
        .document
        .append_element(Some(contents), "style", None::<&str>);
    let detached = host.document.create_detached_element("style").unwrap();
    let mut runtime = DomRuntime::new(host).unwrap();
    ok(
        &mut runtime,
        "var sheets=document.styleSheets;sheets.length===0",
    );
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .pre_insert(head, hidden, None)
            .unwrap()
    })
    .unwrap();
    ok(
        &mut runtime,
        "var hidden=document.head.lastChild;var moved=hidden.sheet;moved!==null && sheets.length===1 && sheets[0]===moved",
    );
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .pre_insert(contents, hidden, None)
            .unwrap()
    })
    .unwrap();
    ok(
        &mut runtime,
        "hidden.sheet===null && sheets.length===0 && sheets.item(0)===null",
    );
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .pre_insert(head, detached, None)
            .unwrap()
    })
    .unwrap();
    ok(&mut runtime, "sheets.length===1 && sheets[0]!==moved");
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .pre_insert(head, hidden, Some(detached))
            .unwrap()
    })
    .unwrap();
    ok(
        &mut runtime,
        "hidden.sheet===moved && sheets.length===2 && sheets[0]===moved",
    );
}

#[test]
fn stale_cached_stylesheet_ids_do_not_root_wrappers_or_payloads() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    ok(
        &mut runtime,
        "var sheets=document.styleSheets,weakSheet1017,weakPayload1017;(()=>{let el=document.createElement('style');document.head.append(el);let sheet=sheets[0];sheet.payload=new Uint8Array(1048576);weakSheet1017=new WeakRef(sheet);weakPayload1017=new WeakRef(sheet.payload);el.remove();})();true",
    );
    runtime.context_mut().clear_kept_objects();
    boa_gc::force_collect();
    ok(
        &mut runtime,
        "weakSheet1017.deref()===undefined && weakPayload1017.deref()===undefined",
    );
    ok(&mut runtime, "sheets.length===0 && sheets.item(0)===null");
}
