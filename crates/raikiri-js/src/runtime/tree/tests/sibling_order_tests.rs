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
                        document.set_element_attribute(node, "id", format!("id{i}")).unwrap();
                    }
                }
                document.mark_in_document_flags();
                host.document = document;
                let mut runtime = DomRuntime::new(host).unwrap();
                runtime.evaluate("var b=document.body; var a=b.children; var warm=a.length; var firstExpected=b.firstChild; var lastExpected=b.lastChild; for(var j=0;j<a.length;j++)a[j];").unwrap();
                let source = match mode {
                    "forward" => format!("(() => {{ let count=0; for(let p=b.firstChild;p;p=p.nextSibling)++count; return count==={n}; }})()"),
                    "backward" => format!("(() => {{ let count=0; for(let p=b.lastChild;p;p=p.previousSibling)++count; return count==={n}; }})()"),
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
            eprintln!("siblings mode={mode} n={n} median_ms={:.6} samples={samples:?}", samples[2]);
        }
    }
}
