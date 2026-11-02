use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;

#[test]
fn margin_names_keep_tokens_brand_positions_and_detached_ownership() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    assert!(
        rt.evaluate(
            r#"
        var styleElement=document.createElement('style');
        styleElement.textContent=String.raw`@page named {
            @TOP-LEFT { content: "left"; }
            @t\6fp-right { content: "right"; }
        }`;
        document.head.appendChild(styleElement);
        var page=styleElement.sheet.cssRules[0];
        var left=page.cssRules[0], right=page.cssRules[1];
        var getter=Object.getOwnPropertyDescriptor(left,'name').get;
        var checked=false;
        try { getter.call({}); } catch(e) { checked=e instanceof TypeError; }
        checked&&left.name==='top-left'&&right.name==='top-right'
    "#
        )
        .unwrap()
        .to_boolean()
    );
    assert!(
        rt.evaluate(
            r#"
        page.insertRule('@bottom-center {content:"new"}',0);
        var followed=page.cssRules[1]===left&&page.cssRules[2]===right;
        page.deleteRule(1);
        var detached=left.name==='top-left'&&left.cssText.includes('left');
        styleElement.textContent='p {}';
        var pageDetached=page.cssRules[1]===right&&right.name==='top-right';
        styleElement=null;page=null;getter=null;
        followed&&detached&&pageDetached
    "#
        )
        .unwrap()
        .to_boolean()
    );
    boa_gc::force_collect();
    assert!(
        rt.evaluate(
            "left.name==='top-left'&&right.name==='top-right'&&right.cssText.includes('right')"
        )
        .unwrap()
        .to_boolean()
    );
}

#[test]
fn a_margin_slot_without_a_range_keeps_the_empty_name_fallback() {
    use super::super::{SheetKey, margin_rule_name, margin_rule_object};
    let (mut host, _, head, _) = StubHost::page();
    let style = host
        .document
        .append_element(Some(head), "style", None::<&str>);
    host.document.append_text(style, "@page { @top-left {} }");
    let mut rt = DomRuntime::new(host).unwrap();
    let margin = margin_rule_object(rt.context_mut(), SheetKey::Element(style), 0, 1).unwrap();
    let name = margin_rule_name(&margin.into(), &[], rt.context_mut()).unwrap();
    assert_eq!(name.as_string().unwrap().to_std_string_escaped(), "");
}
