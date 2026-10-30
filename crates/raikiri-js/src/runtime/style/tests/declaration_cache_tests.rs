use super::super::{
    ValidatedDeclaration, declaration_value, parse_inline_style, same_property, shorthand,
    validated_inline_declarations, validated_property_name,
};
use super::*;
use crate::css_declaration_diagnostics as diagnostics;
use crate::runtime::webidl::with_state;

fn custom_text(n: usize) -> String {
    (0..n).map(|i| format!("--p{i:06}:0;")).collect()
}

#[test]
fn single_validation_has_bounded_duplicate_removal_work() {
    for n in [32, 64, 128] {
        let text = custom_text(n);
        diagnostics::begin();
        let declarations = validated_inline_declarations(Some(&text), false);
        let work = diagnostics::finish();
        assert_eq!(declarations.len(), n);
        assert_eq!(work.validation_calls, 1);
        assert_eq!(work.raw_declarations, n as u64);
        assert!(
            work.linear_name_comparisons
                + work.index_lookups
                + work.index_removals
                + work.index_insertions
                <= 4 * n as u64,
            "n={n}, work={work:?}"
        );
    }
}

#[test]
fn mixed_unchanged_reads_share_one_validation() {
    for n in [32, 64, 128] {
        let (mut host, _, _, body) = StubHost::page();
        host.document
            .set_element_inline_style(body, Some(custom_text(n).into()));
        let mut rt = DomRuntime::new(host).unwrap();
        rt.evaluate("globalThis.s=document.body.style;").unwrap();
        diagnostics::begin();
        ok(
            &mut rt,
            &format!(
                "(()=>{{const n=s.length; let count=0;for(let i=0;i<n;i++)count+=s.item(i).length;return n==={n} && count==={} && s.getPropertyValue('--p000000')==='0' && s.getPropertyPriority('--p000000')==='' && s.cssText.includes('--p000000: 0;');}})()",
                n * 9
            ),
        );
        let work = diagnostics::finish();
        assert_eq!(work.validation_calls, 1, "{work:?}");
        assert_eq!(work.raw_declarations, n as u64);
        diagnostics::begin();
        ok(
            &mut rt,
            "s.length>0 && s[0]==='--p000000' && s.getPropertyValue('--p000000')==='0' && s.getPropertyPriority('--p000000')==='' && s.cssText.includes('--p000000: 0;')",
        );
        assert_eq!(diagnostics::finish().validation_calls, 0);
    }
}

fn declaration_tuples(declarations: Vec<ValidatedDeclaration>) -> Vec<(String, String, bool)> {
    declarations
        .into_iter()
        .map(|d| (d.name, d.value, d.important))
        .collect()
}

// The pre-index algorithm is an independent ordering oracle for the new dedup.
fn reference_validation(text: &str, page: bool) -> Vec<(String, String, bool)> {
    let mut out: Vec<ValidatedDeclaration> = Vec::new();
    for raw in parse_inline_style(text) {
        let Some(name) = validated_property_name(&raw.name, page) else {
            continue;
        };
        if raw.value.is_empty() {
            continue;
        }
        let Some(stored) = declaration_value(&name, &raw.value, page) else {
            continue;
        };
        if stored.trim().is_empty() {
            continue;
        }
        if let Some(longhands) = shorthand::longhands_of(&name) {
            out.retain(|d| !same_property(&d.name, &name) && !longhands.contains(&d.name.as_str()));
        }
        for (name, value) in
            shorthand::expand(&name, &stored, page).unwrap_or_else(|| vec![(name, stored)])
        {
            out.retain(|d| !same_property(&d.name, &name));
            out.push(ValidatedDeclaration {
                name,
                value,
                important: raw.important,
            });
        }
    }
    declaration_tuples(out)
}

#[test]
fn indexed_dedup_matches_last_occurrence_and_shorthand_oracle() {
    let choices = [
        "--A:1;",
        "--a:2;",
        "--A:3!important;",
        "--a:4;",
        "COLOR:red;",
        "color:blue!important;",
        "color:bogus;",
        "width:1px;",
        "width:;",
        "width:bogus;",
        "background-color:red;",
        "background:none;",
        "background-image:none;",
        "background:var(--bg);",
        "background:bogus;",
        "margin:1px;",
        "margin-left:2px;",
        "margin:var(--m);",
        "size:A4;",
        r"--\41 :5;",
        "unknown:1;",
    ];
    let mut random = 17_u64;
    for page in [false, true] {
        for _ in 0..40 {
            let mut text = String::new();
            for _ in 0..40 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                text.push_str(choices[(random >> 32) as usize % choices.len()]);
            }
            assert_eq!(
                declaration_tuples(validated_inline_declarations(Some(&text), page)),
                reference_validation(&text, page),
                "page={page} text={text}"
            );
        }
    }
}

#[test]
fn cached_declarations_observe_js_mutations_and_invalid_values() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "var e=document.body; var s=e.style; e.setAttribute('style','--A:1; --a:2; color:red;');",
    )
    .unwrap();
    ok(
        &mut rt,
        "s.length===3 && s.item(0)==='--A' && s.cssText.includes('color: red;')",
    );
    rt.evaluate("s.setProperty('--A','3','important');")
        .unwrap();
    ok(
        &mut rt,
        "s.length===3 && s.item(2)==='--A' && s.getPropertyValue('--A')==='3' && s.getPropertyPriority('--A')==='important'",
    );
    rt.evaluate("s.removeProperty('--a');").unwrap();
    ok(&mut rt, "s.length===2 && s.getPropertyValue('--a')===''");
    rt.evaluate("s.cssText='background-color:red; background:var(--b); width:1px; width:bogus;';")
        .unwrap();
    ok(
        &mut rt,
        "s.getPropertyValue('background-color')==='' && s.getPropertyValue('background')==='var(--b)' && s.getPropertyValue('width')==='1px'",
    );
    rt.evaluate("e.removeAttribute('style');").unwrap();
    ok(&mut rt, "s.length===0 && s.cssText===''");
    rt.evaluate("e.setAttribute('style','');").unwrap();
    ok(&mut rt, "s.length===0 && s.cssText===''");
    rt.evaluate("e.setAttribute('style','--x:7;');").unwrap();
    ok(&mut rt, "s.length===1 && s.getPropertyValue('--x')==='7'");
}

#[test]
fn cached_declarations_observe_native_setters_equal_replacement_and_document_change() {
    let (mut host, _, _, body) = StubHost::page();
    let original = format!("{}--tail:1;", custom_text(32));
    host.document
        .set_element_inline_style(body, Some(original.clone().into()));
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var s=document.body.style;").unwrap();
    ok(
        &mut rt,
        "s.length===33 && s.getPropertyValue('--tail')==='1'",
    );
    with_state(rt.context_mut(), |state| {
        state
            .host
            .document_mut()
            .set_element_inline_style(body, Some(original.clone().into()))
    })
    .unwrap();
    ok(
        &mut rt,
        "s.length===33 && s.getPropertyValue('--tail')==='1'",
    );
    with_state(rt.context_mut(), |state| {
        state.host.document_mut().set_element_attribute(
            body,
            "style",
            original.replace("--tail:1;", "--tail:2;"),
        )
    })
    .unwrap()
    .unwrap();
    ok(
        &mut rt,
        "s.length===33 && s.getPropertyValue('--tail')==='2'",
    );
    with_state(rt.context_mut(), |state| {
        state
            .host
            .document_mut()
            .remove_element_attribute(body, "style")
    })
    .unwrap()
    .unwrap();
    ok(&mut rt, "s.length===0 && s.getPropertyValue('--tail')===''");
    let (mut replacement, ..) = StubHost::page();
    replacement
        .document
        .set_element_inline_style(body, Some("--new:3;".into()));
    with_state(rt.context_mut(), |state| {
        state
            .host
            .as_any_mut()
            .downcast_mut::<StubHost>()
            .unwrap()
            .document = replacement.document
    })
    .unwrap();
    ok(&mut rt, "s.length===1 && s.getPropertyValue('--new')==='3'");
}

#[test]
fn cached_declarations_follow_rule_movement_and_detached_writes() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var st=document.createElement('style');st.textContent='p{--x:1;}';document.head.appendChild(st);var sheet=st.sheet;var rule=sheet.cssRules[0];var old=rule.style;").unwrap();
    ok(
        &mut rt,
        "old.length===1 && old.getPropertyValue('--x')==='1'",
    );
    rt.evaluate("sheet.insertRule('div{--y:2;}',0);").unwrap();
    ok(&mut rt, "sheet.cssRules[1]===rule && old.item(0)==='--x'");
    rt.evaluate("sheet.deleteRule(1);old.setProperty('--x','3');sheet.insertRule('p{--x:4;}',1);var fresh=sheet.cssRules[1].style;").unwrap();
    ok(
        &mut rt,
        "old.length===1 && old.getPropertyValue('--x')==='3' && fresh.getPropertyValue('--x')==='4'",
    );
    rt.evaluate("fresh.cssText='--x:5;';old.cssText='--x:6;--z:7;';")
        .unwrap();
    ok(
        &mut rt,
        "old.length===2 && old.item(1)==='--z' && old.getPropertyValue('--x')==='6' && fresh.length===1 && fresh.getPropertyValue('--x')==='5'",
    );
}

#[test]
fn cache_retains_only_compact_survivors_and_adopts_equal_replacement_snapshot() {
    use super::super::{DeclarationText, StyleSource};
    use std::cell::RefCell;
    let (mut host, _, _, body) = StubHost::page();
    let original = "--same:abcdefghijklmnopqrstuvwxyz;".repeat(2000);
    host.document
        .set_element_inline_style(body, Some(original.clone().into()));
    let mut rt = DomRuntime::new(host).unwrap();
    let source = StyleSource {
        index: body,
        computed: false,
        rule: None,
        declarations: RefCell::new(None),
    };
    with_state(rt.context_mut(), |state| {
        source.with_declarations(state, |declarations| assert_eq!(declarations.len(), 1))
    })
    .unwrap();
    with_state(rt.context_mut(), |state| {
        state
            .host
            .document_mut()
            .set_element_inline_style(body, Some(original.clone().into()))
    })
    .unwrap();
    diagnostics::begin();
    with_state(rt.context_mut(), |state| {
        source.with_declarations(state, |declarations| assert_eq!(declarations.len(), 1))
    })
    .unwrap();
    assert_eq!(diagnostics::finish().validation_calls, 0);
    let current = with_state(rt.context_mut(), |state| {
        state
            .host
            .document()
            .element_inline_style_snapshot(body)
            .unwrap()
    })
    .unwrap();
    {
        let cache = source.declarations.borrow();
        let entry = cache.as_ref().unwrap();
        assert_eq!(
            std::mem::size_of_val(&*entry.declarations),
            std::mem::size_of::<ValidatedDeclaration>()
        );
        let DeclarationText::Inline(Some(key)) = &entry.text else {
            panic!("expected inline snapshot")
        };
        assert_eq!(key.as_ptr(), current.as_ptr());
    }
    with_state(rt.context_mut(), |state| {
        state
            .host
            .document_mut()
            .set_element_inline_style(body, Some("--new:2;".into()))
    })
    .unwrap();
    with_state(rt.context_mut(), |state| {
        source.with_declarations(state, |declarations| {
            assert_eq!(declarations[0].name, "--new")
        })
    })
    .unwrap();
    let cache = source.declarations.borrow();
    let entry = cache.as_ref().unwrap();
    assert_eq!(entry.declarations.len(), 1);
    assert_eq!(entry.text.parts(), (Some("--new:2;"), false));
}

#[test]
fn page_descriptor_cache_tracks_page_and_margin_rule_updates() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("var st=document.createElement('style');st.textContent='@page {size:A4; @top-left {content:\"x\";}}';document.head.appendChild(st);var page=st.sheet.cssRules[0];var p=page.style;var m=page.cssRules[0].style;").unwrap();
    ok(
        &mut rt,
        "p.length===1 && p.item(0)==='size' && p.getPropertyValue('size')==='A4' && m.length===1 && m.getPropertyValue('content')==='\"x\"'",
    );
    rt.evaluate("p.cssText='size:letter;';m.setProperty('content','\"y\"');")
        .unwrap();
    ok(
        &mut rt,
        "p.length===1 && p.getPropertyValue('size')==='letter' && m.length===1 && m.getPropertyValue('content')==='\"y\"'",
    );
}
