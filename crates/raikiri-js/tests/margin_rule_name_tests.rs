//! Public allocation guards for margin-rule names in large rule bodies.
#[path = "support/allocation_tests.rs"]
mod support;
#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;
#[test]
fn margin_names_do_not_copy_the_rule_body() {
    let mut readings = Vec::new();
    for size in [4096, 65536, 1048576] {
        let mut runtime = support::runtime();
        runtime.evaluate(&format!(r#"var html=document.createElement('html');document.appendChild(html);var head=document.createElement('head');html.appendChild(head);var style=document.createElement('style');style.textContent='@page target {{ @TOP-LEFT {{ content: "'+ 'x'.repeat({size}) +'"; }} }} @page control {{ @top-left {{ content: "a"; }} }}';head.appendChild(style);globalThis.large=style.sheet.cssRules[0].cssRules[0];globalThis.small=style.sheet.cssRules[1].cssRules[0];large.name==='top-left'&&small.name==='top-left';"#)).unwrap();
        let mut totals = Vec::new();
        for subject in ["large", "small"] {
            let script = format!(
                "(()=>{{let hits=0;for(let i=0;i<100;i++)if({subject}.name==='top-left')hits++;return hits===100;}})()"
            );
            let (result, allocation) = support::allocated_by(|| runtime.evaluate(&script));
            assert!(result.unwrap().to_boolean());
            eprintln!("margin name: size={size}, subject={subject}, {allocation:?}");
            totals.push(allocation.bytes);
        }
        readings.push((size, totals[0], totals[1]));
    }
    for (size, large, small) in readings {
        assert!(
            large.abs_diff(small) < 1024,
            "size={size},large={large},small={small}"
        );
        assert!(large < 256 * 1024, "size={size},bytes={large}");
    }
}
