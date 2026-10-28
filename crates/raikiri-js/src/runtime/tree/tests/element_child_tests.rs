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
                    document.set_element_attribute(node, "id", format!("id{i}")).unwrap();
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
                let source = format!("(() => {{let value;for(let i=0;i<{n};++i)value=b.{property};return value==={expected};}})()");
                let start = Instant::now();
                let result = runtime.evaluate(&source).unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1_000.0);
                assert!(result.to_boolean());
                drop(runtime);
                boa_gc::force_collect();
            }
            samples.sort_by(f64::total_cmp);
            eprintln!("element child mode={mode} n={n} queries={n} median_ms={:.6} samples={samples:?}", samples[2]);
        }
    }
}
