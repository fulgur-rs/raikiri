use super::*;
use crate::runtime::{RunOptions, RuntimeError};
use std::time::Instant;

pub(super) fn run(options: RunOptions, label: &str) -> DomRuntime {
    let number = |key: &str, default| {
        std::env::var(key)
            .ok()
            .map(|s| s.parse().unwrap())
            .unwrap_or(default)
    };
    let n = number("SELECTOR_NATIVE_N", 65_537usize);
    let parents = number("SELECTOR_NATIVE_PARENTS", 1);
    let candidates = number("SELECTOR_NATIVE_MATCH_CANDIDATES", n);
    let filters = number("SELECTOR_NATIVE_FILTERS", 1);
    let repeats = number("SELECTOR_NATIVE_REPEATS", 1);
    let mode = std::env::var("SELECTOR_NATIVE_MODE").unwrap_or_else(|_| "all".into());
    let (mut host, _, _, body) = StubHost::page();
    let mut first = 0;
    for _ in 0..parents {
        let parent = host.document.append_element(Some(body), "ul", None::<&str>);
        for i in 0..n {
            let child = host
                .document
                .append_element(Some(parent), "li", None::<&str>);
            host.document
                .set_element_attribute(child, "class", if i % 3 == 0 { "visible" } else { "other" })
                .unwrap();
            if first == 0 {
                first = child;
            }
        }
    }
    let selector = if filters == 1 {
        if candidates < n {
            format!("li:nth-child(-n+{candidates}):nth-child(n of .visible)")
        } else {
            "li:nth-child(n of .visible)".to_owned()
        }
    } else {
        (0..filters)
            .map(|i| format!("li:nth-child(n of .missing{i})"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let expected = if filters > 1 {
        0
    } else if mode == "prefix" {
        repeats
    } else {
        n.min(candidates).div_ceil(3) * parents * repeats
    };
    let mut rt = DomRuntime::with_options(host, options).unwrap();
    expose(&mut rt, "subject", body);
    expose(&mut rt, "probeFirst", first);
    let operation = if mode == "prefix" {
        format!("Number(probeFirst.matches('{selector}'))")
    } else {
        format!("subject.querySelectorAll('{selector}').length")
    };
    let script = format!(
        "var probeHits=0; for(var probeIndex=0; probeIndex<{repeats}; probeIndex++){{probeHits += {operation};}} probeHits"
    );
    let start = Instant::now();
    let result = rt.evaluate(&script);
    let elapsed = start.elapsed();
    let stopped = match result {
        Ok(value) => {
            assert_eq!(value.as_number().unwrap() as usize, expected);
            false
        }
        Err(RuntimeError::Aborted(reason)) => {
            assert_eq!(reason.to_string(), "selector work limit reached");
            true
        }
        Err(other) => panic!("unexpected native query error: {other:?}"),
    };
    eprintln!(
        "SELECTOR_NATIVE label={label} mode={mode} n={n} candidates={candidates} parents={parents} filters={filters} repeats={repeats} elapsed_ns={} stopped={stopped}",
        elapsed.as_nanos()
    );
    rt
}

#[test]
#[ignore = "manual production query work/time/RSS probe, baseline compatible"]
fn native_selector_work_probe() {
    run(RunOptions::default(), "default");
}
