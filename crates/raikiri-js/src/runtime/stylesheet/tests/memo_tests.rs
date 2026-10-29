use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;

use super::take_sheet_work;
use crate::runtime::webidl::with_state;

#[test]
fn immutable_css_rule_reads_reuse_source_and_parse() {
    let (mut host, _, head, _) = StubHost::page();
    let element = host
        .document
        .append_element(Some(head), "style", None::<&str>);
    let css: String = (0..64).map(|i| format!(".c{i}{{color:red}}\n")).collect();
    host.document.append_text(element, css);
    let mut runtime = DomRuntime::new(host).unwrap();
    assert!(runtime.evaluate("var rules=document.styleSheets[0].cssRules;var held=[];for(var i=0;i<64;++i)held.push(rules.item(i));rules.length===64").unwrap().to_boolean());
    for expression in [
        "rules.length===64",
        "rules.item(i)===held[i] && held[i].selectorText==='.c'+i",
        "held[i].cssText.includes('.c'+i)",
    ] {
        take_sheet_work();
        let script = format!(
            "(()=>{{for(let i=0;i<64;++i)if(!({expression}))return false;return true;}})()"
        );
        assert!(runtime.evaluate(&script).unwrap().to_boolean());
        assert_eq!(take_sheet_work(), [0, 0]);
    }
}

#[test]
fn length_and_out_of_range_items_do_not_retain_versions_for_every_sheet() {
    let (mut host, _, head, _) = StubHost::page();
    for i in 0..64 {
        let element = host
            .document
            .append_element(Some(head), "style", None::<&str>);
        host.document
            .append_text(element, format!(".c{i}{{color:red}}"));
    }
    let mut runtime = DomRuntime::new(host).unwrap();
    assert!(runtime.evaluate("var sheets=document.styleSheets;(()=>{for(let i=0;i<64;++i){let r=sheets[i].cssRules;if(r.length!==1||r.item(99)!==null)return false;}return true;})()").unwrap().to_boolean());
    assert_eq!(
        with_state(runtime.context_mut(), |s| s.css_sheet_versions.len()).unwrap(),
        0
    );
    take_sheet_work();
    assert!(
        runtime
            .evaluate("sheets[63].cssRules.length===1 && sheets[63].cssRules.item(99)===null")
            .unwrap()
            .to_boolean()
    );
    assert_eq!(take_sheet_work(), [0, 0]);
    assert!(
        runtime
            .evaluate(
                "var r=sheets[63].cssRules[0];r.selectorText==='.c63' && sheets[63].cssRules[0]===r"
            )
            .unwrap()
            .to_boolean()
    );
    assert_eq!(
        with_state(runtime.context_mut(), |s| s.css_sheet_versions.len()).unwrap(),
        1
    );
    assert!(runtime.evaluate("document.head.lastChild.textContent='.fresh{}';r.selectorText==='.c63' && sheets[63].cssRules[0].selectorText==='.fresh' && sheets[63].cssRules[0]!==r").unwrap().to_boolean());
}

fn runtime_with_sheet(css: &str) -> (DomRuntime, usize, usize) {
    let (mut host, _, head, _) = StubHost::page();
    let element = host
        .document
        .append_element(Some(head), "style", None::<&str>);
    let text = host.document.append_text(element, css);
    (DomRuntime::new(host).unwrap(), element, text)
}

fn ok(runtime: &mut DomRuntime, script: &str) {
    assert!(runtime.evaluate(script).unwrap().to_boolean(), "{script}");
}

#[test]
fn native_text_source_and_document_replacement_refresh_without_js_generation() {
    let (mut runtime, element, text) = runtime_with_sheet("p{color:red}");
    ok(
        &mut runtime,
        "var list=document.styleSheets[0].cssRules;var original=list[0];original.selectorText==='p'",
    );
    let generation = with_state(runtime.context_mut(), |s| s.generation).unwrap();
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .set_character_data(text, "a{color:blue}")
            .unwrap()
    })
    .unwrap();
    ok(
        &mut runtime,
        "var fresh=list[0];fresh!==original && fresh.selectorText==='a' && original.selectorText==='p' && original.style.color==='red'",
    );
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .append_text_coalesced(element, " b{color:green}")
    })
    .unwrap();
    ok(
        &mut runtime,
        "list.length===2 && list[1].selectorText==='b' && list[0]!==fresh && fresh.style.color==='blue'",
    );
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .set_stylesheet_source(element, "q{color:purple}".into())
            .unwrap()
    })
    .unwrap();
    ok(
        &mut runtime,
        "list.length===1 && list[0].selectorText==='q' && document.head.firstChild.textContent.includes('color:blue')",
    );
    with_state(runtime.context_mut(), |s| {
        let mut replacement = s.host.document().clone();
        replacement
            .set_stylesheet_source(element, "i{color:orange}".into())
            .unwrap();
        *s.host.document_mut() = replacement;
    })
    .unwrap();
    ok(
        &mut runtime,
        "list.length===1 && list[0].selectorText==='i' && original.selectorText==='p'",
    );
    assert_eq!(
        with_state(runtime.context_mut(), |s| s.generation).unwrap(),
        generation
    );
}

#[test]
fn unrelated_native_mutation_rechecks_without_detaching_or_reparsing_the_sheet() {
    let (mut runtime, _, _) = runtime_with_sheet("p{color:red}");
    ok(
        &mut runtime,
        "var list=document.styleSheets[0].cssRules;var held=list[0];held.selectorText==='p'",
    );
    take_sheet_work();
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .append_element(Some(0), "div", None::<&str>)
    })
    .unwrap();
    ok(&mut runtime, "list[0]===held && held.selectorText==='p'");
    assert_eq!(take_sheet_work(), [0, 1]);
    ok(
        &mut runtime,
        "list[0]===held && list.length===1 && held.style.color==='red'",
    );
    assert_eq!(take_sheet_work(), [0, 0]);
}

#[test]
fn same_text_replacement_and_detached_rule_writes_keep_separate_contexts() {
    let (mut runtime, _, _) = runtime_with_sheet("p{color:red}");
    ok(
        &mut runtime,
        "var el=document.head.firstChild;var sheet=el.sheet;var original=sheet.cssRules[0];el.textContent=el.textContent;var fresh=sheet.cssRules[0];original!==fresh && original.selectorText==='p'",
    );
    ok(
        &mut runtime,
        "sheet.deleteRule(0);fresh.style.color='blue';fresh.style.color==='blue' && fresh.cssText.includes('blue') && original.style.color==='red' && sheet.cssRules.length===0",
    );
    ok(
        &mut runtime,
        "fresh.style.color='green';fresh.cssText.includes('green') && fresh.style.color==='green'",
    );
    take_sheet_work();
    ok(
        &mut runtime,
        "fresh.cssText.includes('green') && fresh.selectorText==='p' && fresh.style.color==='green'",
    );
    assert_eq!(take_sheet_work(), [0, 0]);
}

#[test]
fn immutable_page_and_margin_reads_share_the_memo_through_shifts_and_detachment() {
    let (mut runtime, _, _) =
        runtime_with_sheet("@page {size:A4; @top-left {content:'x';color:red}} p{color:blue}");
    ok(
        &mut runtime,
        "var sheet=document.styleSheets[0];var list=sheet.cssRules;var page=list[0];var margin=page.cssRules[0];var ps=page.style;var ms=margin.style;page.cssRules.length===1 && margin.name==='top-left' && ms.color==='red' && ps.cssText.includes('size')",
    );
    take_sheet_work();
    ok(
        &mut runtime,
        "(()=>{for(let i=0;i<64;++i)if(list.length!==2||list[0]!==page||page.cssRules.length!==1||page.cssRules[0]!==margin||margin.name!=='top-left'||!margin.cssText.includes('red')||ms.color!=='red'||!ps.cssText.includes('size'))return false;return true;})()",
    );
    assert_eq!(take_sheet_work(), [0, 0]);
    ok(
        &mut runtime,
        "page.insertRule('@bottom-right {content: \"y\"}',0);page.cssRules.length===2 && page.cssRules[1]===margin",
    );
    ok(&mut runtime, "page.deleteRule(0);page.cssRules[0]===margin");
    ok(
        &mut runtime,
        "sheet.deleteRule(0);margin.style.color='green';page.cssRules[0]===margin && margin.style.color==='green' && list.length===1 && list[0].selectorText==='p'",
    );
}

#[test]
fn memo_text_and_metadata_do_not_root_detached_rules_or_payloads() {
    let (mut runtime, _, _) = runtime_with_sheet("p{}");
    ok(
        &mut runtime,
        "var weakRule1016,weakPayload1016;(()=>{let el=document.createElement('style');el.textContent='a{color:red}';document.head.append(el);let rule=el.sheet.cssRules[0];rule.payload=new Uint8Array(1048576);if(rule.selectorText!=='a')throw new Error('rule');weakRule1016=new WeakRef(rule);weakPayload1016=new WeakRef(rule.payload);el.remove();})();true",
    );
    runtime.context_mut().clear_kept_objects();
    boa_gc::force_collect();
    ok(
        &mut runtime,
        "weakRule1016.deref()===undefined && weakPayload1016.deref()===undefined",
    );
}
