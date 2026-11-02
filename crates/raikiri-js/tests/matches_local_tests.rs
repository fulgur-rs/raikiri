//! Public allocation guards for local selectors on deep element chains.
#[path = "support/allocation_tests.rs"]
mod support;
use raikiri_dom::Document;
use raikiri_js::runtime::{DomRuntime, Limits, RunOptions};
#[global_allocator]
static ALLOCATOR: support::CountingAllocator = support::CountingAllocator;
#[test]
fn local_matches_does_not_allocate_an_ancestor_chain() {
    let mut readings = Vec::new();
    for depth in [100, 1000, 8000] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", None::<&str>);
        let body = doc.append_element(Some(html), "body", None::<&str>);
        let mut leaf = body;
        for _ in 0..depth {
            leaf = doc.append_element(Some(leaf), "div", None::<&str>);
        }
        doc.set_element_attribute(leaf, "id", "leaf").unwrap();
        doc.set_element_attribute(leaf, "class", "probe").unwrap();
        doc.set_element_attribute(leaf, "data-probe", "yes")
            .unwrap();
        doc.mark_in_document_flags();
        let mut runtime = DomRuntime::with_options(
            support::Host(doc),
            RunOptions {
                limits: Limits {
                    max_selector_work: None,
                    ..Limits::default()
                },
            },
        )
        .unwrap();
        runtime
            .evaluate("globalThis.subject=document.getElementById('leaf');subject.matches('*')")
            .unwrap();
        for selector in [
            "*",
            "div",
            ".probe",
            "#leaf",
            "[data-probe='yes']",
            "div.probe#leaf[data-probe]",
        ] {
            let script = format!(
                r#"(()=>{{let hits=0;for(let i=0;i<1000;i++)if(subject.matches("{selector}"))hits++;return hits===1000;}})()"#
            );
            let (result, allocation) = support::allocated_by(|| runtime.evaluate(&script));
            assert!(result.unwrap().to_boolean());
            eprintln!("local matches: depth={depth}, selector={selector}, {allocation:?}");
            readings.push((depth, selector, allocation.bytes));
        }
        let (result, allocation) = support::allocated_by(|| {
            runtime.evaluate("(()=>{let hits=0;for(let i=0;i<1000;i++)if(document.documentElement.matches('*'))hits++;return hits===1000;})()")
        });
        assert!(result.unwrap().to_boolean());
        eprintln!("root matches: depth={depth}, {allocation:?}");
        readings.push((depth, "root *", allocation.bytes));
    }
    let mut detached = support::runtime();
    detached
        .evaluate("globalThis.subject=document.createElement('div');")
        .unwrap();
    let (result, allocation) = support::allocated_by(|| {
        detached.evaluate("(()=>{let hits=0;for(let i=0;i<1000;i++)if(subject.matches('*'))hits++;return hits===1000;})()")
    });
    assert!(result.unwrap().to_boolean());
    eprintln!("detached matches: {allocation:?}");
    readings.push((0, "detached *", allocation.bytes));
    for (depth, selector, bytes) in readings {
        assert!(
            bytes < 2 * 1024 * 1024,
            "depth={depth},selector={selector},bytes={bytes}"
        );
    }
}
