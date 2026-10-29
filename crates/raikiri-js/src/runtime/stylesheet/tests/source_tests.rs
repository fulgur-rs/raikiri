use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;

#[test]
fn cssom_updates_keep_the_original_text_node_and_bound_native_history() {
    let (mut host, _, head, _) = StubHost::page();
    let element = host
        .document
        .append_element(Some(head), "style", None::<&str>);
    let original = "p { color: red }";
    let text = host.document.append_text(element, original);
    let count = host.document.node_count();
    let mut runtime = DomRuntime::new(host).unwrap();
    runtime
        .evaluate(
            "var el = document.head.firstChild; var text = el.firstChild; \
        var sheet = el.sheet; var rule = sheet.cssRules[0]; var style = rule.style; \
        for (var i=0; i<100; i++) sheet.insertRule('.c'+i+' { color: red }', i+1); \
        for (var i=0; i<100; i++) style.color = i%2 ? 'blue' : 'green';",
        )
        .unwrap();
    let host = runtime.into_host();
    assert_eq!(host.document().node_count(), count);
    assert_eq!(host.document().parent_of(text), Some(element));
    assert_eq!(
        host.document().element_text_content(element).as_deref(),
        Some(original)
    );
}

#[test]
fn cssom_writes_preserve_dom_text_and_external_character_data_replaces_sheet() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    let result = runtime
        .evaluate(
            "var el = document.createElement('style'); \
        el.textContent = 'p { color: red }'; document.head.appendChild(el); \
        var text = el.firstChild, sheet = el.sheet, rule = sheet.cssRules[0], style = rule.style; \
        style.color = 'blue'; sheet.insertRule('div {}', 1); sheet.deleteRule(1); \
        var preserved = el.firstChild === text && text.parentNode === el && \
            text.data === 'p { color: red }' && el.textContent === text.data && \
            sheet.cssRules[0] === rule && rule.style === style && style.color === 'blue'; \
        text.data = 'p { color: green }'; \
        var fresh = sheet.cssRules[0]; style.color = 'white'; \
        preserved && fresh !== rule && fresh.style.color === 'green' && style.color === 'white' \
            && text.data === 'p { color: green }'",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn empty_sources_clone_and_reconnection_follow_dom_sheet_lifetimes() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    let value = runtime
        .evaluate(
            "var el = document.createElement('style'); \
        document.head.appendChild(el); var sheet = el.sheet; \
        sheet.insertRule('p { color: blue }'); var rule = sheet.cssRules[0]; \
        var clone = el.cloneNode(true); document.head.appendChild(clone); \
        var emptyClone = clone.sheet.cssRules.length === 0 && clone.textContent === ''; \
        sheet.deleteRule(0); var empty = sheet.cssRules.length === 0; \
        sheet.insertRule('p { color: green }'); var edited = sheet.cssRules[0]; \
        el.remove(); document.head.appendChild(el); \
        emptyClone && empty && el.sheet.cssRules.length === 0 && \
        edited.style.color === 'green' && rule.style.color === 'blue' && el.textContent === ''",
        )
        .unwrap();
    assert!(value.to_boolean());
}

#[test]
fn empty_cssom_source_does_not_revive_original_rules() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    let value = runtime
        .evaluate(
            "var el = document.createElement('style'); \
        el.textContent = 'p { color: red }'; document.head.appendChild(el); \
        var sheet = el.sheet, old = sheet.cssRules[0]; sheet.deleteRule(0); \
        var empty = sheet.cssRules.length === 0 && el.textContent === 'p { color: red }'; \
        el.firstChild.appendData(''); var fresh = sheet.cssRules[0]; \
        empty && fresh !== old && fresh.style.color === 'red'",
        )
        .unwrap();
    assert!(value.to_boolean());
}

/// Reproduce native arena retention with the normal runtime allocator.
/// Run in release mode with `--ignored --nocapture` on an otherwise idle CPU.
#[test]
#[ignore]
fn cssom_source_retention_probe() {
    use std::time::Instant;
    for (initial, updates, insert) in [
        (0, 250, true),
        (0, 500, true),
        (0, 1000, true),
        (0, 2000, true),
        (1000, 100, false),
        (1000, 200, false),
        (1000, 400, false),
        (1000, 800, false),
    ] {
        let (mut host, _, head, _) = StubHost::page();
        let element = host
            .document
            .append_element(Some(head), "style", None::<&str>);
        let initial_text: String = (0..initial)
            .map(|i| format!(".c{i}{{color:red}}\n"))
            .collect();
        host.document.append_text(element, initial_text);
        let mut runtime = DomRuntime::new(host).unwrap();
        runtime.evaluate("var sheet=document.styleSheets[0], rules=sheet.cssRules; var warm=rules.length; var style=rules.length ? rules[0].style : null;").unwrap();
        let script = if insert {
            format!("for(var i=0;i<{updates};i++) sheet.insertRule('.c'+i+'{{color:red}}',i);")
        } else {
            format!("for(var i=0;i<{updates};i++) style.color=i%2?'red':'blue';")
        };
        let start = Instant::now();
        runtime.evaluate(&script).unwrap();
        let elapsed = start.elapsed();
        let host = runtime.into_host();
        let doc = host.document();
        let (mut detached, mut bytes) = (0, 0);
        for id in 0..doc.node_count() {
            let node = doc.get_node(id).unwrap();
            if doc.parent_of(id).is_none()
                && let Some(text) = node.text_content()
            {
                detached += 1;
                bytes += text.len();
            }
        }
        let current = crate::runtime::stylesheet::stylesheet_text(doc, element).len();
        eprintln!(
            "cssom_source_probe initial={initial} updates={updates} insert={insert} nodes={} detached={detached} detached_bytes={bytes} current_bytes={current} elapsed_ms={:.4}",
            doc.node_count(),
            elapsed.as_secs_f64() * 1000.0
        );
    }
}
