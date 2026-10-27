use super::*;
use crate::runtime::webidl::with_state_unchecked;
use crate::runtime::{Abort, Limits, RunOptions, RuntimeError};

fn bounded_runtime(limit: Option<u64>) -> (DomRuntime, usize) {
    let (host, _, _, body) = StubHost::page();
    let mut rt = DomRuntime::with_options(
        host,
        RunOptions {
            limits: Limits {
                max_selector_work: limit,
                ..Limits::default()
            },
        },
    )
    .unwrap();
    expose(&mut rt, "subject", body);
    (rt, body)
}

fn used(rt: &mut DomRuntime) -> u64 {
    with_state_unchecked(rt.context_mut(), |s| s.event_loop.selector_work.count()).unwrap()
}

fn global_bool(rt: &mut DomRuntime, name: &str) -> bool {
    let context = rt.context_mut();
    context
        .global_object()
        .get(JsString::from(name), context)
        .unwrap()
        .to_boolean()
}

#[test]
fn all_four_native_entrypoints_abort_without_partial_results_or_later_effects() {
    for expression in [
        "document.querySelector('*')",
        "document.querySelectorAll('*')",
        "subject.matches('*')",
        "subject.closest('*')",
    ] {
        let (mut rt, _) = bounded_runtime(Some(0));
        rt.evaluate("var timerRan=false; setTimeout(()=>timerRan=true,0)")
            .unwrap();
        let result = rt.evaluate(&format!("var returned=false,caught=false,finalized=false; try {{ {expression}; returned=true; }} catch(e) {{caught=true}} finally {{finalized=true}}"));
        assert_eq!(
            result,
            Err(RuntimeError::Aborted(Abort::SelectorWork)),
            "{expression}"
        );
        assert_eq!(used(&mut rt), 0);
        for name in ["returned", "caught", "finalized", "timerRan"] {
            assert!(!global_bool(&mut rt, name), "{expression}: {name}");
        }
        assert_eq!(rt.run_until_idle(), Err(Abort::SelectorWork));
        assert_eq!(
            rt.evaluate("document.body.appendChild(document.createElement('b'))"),
            Err(RuntimeError::Aborted(Abort::SelectorWork))
        );
    }
}

#[test]
fn different_queries_and_repeated_evaluations_share_one_exact_runtime_budget() {
    let scripts = [
        "subject.matches('*')",
        "subject.closest('html') === document.documentElement",
        "document.querySelector('body') === subject",
        "document.querySelectorAll('body').length === 1",
    ];
    let (mut unlimited, _) = bounded_runtime(None);
    for script in scripts {
        ok(&mut unlimited, script);
    }
    let total = used(&mut unlimited);
    assert!(total > 10);
    let (mut exact, _) = bounded_runtime(Some(total));
    for script in scripts {
        ok(&mut exact, script);
    }
    assert_eq!(used(&mut exact), total);
    assert_eq!(
        exact.evaluate("subject.matches('*')"),
        Err(RuntimeError::Aborted(Abort::SelectorWork))
    );
    let (mut short, _) = bounded_runtime(Some(total - 1));
    for script in &scripts[..3] {
        ok(&mut short, script);
    }
    assert_eq!(
        short.evaluate(scripts[3]),
        Err(RuntimeError::Aborted(Abort::SelectorWork))
    );
    assert!(used(&mut short) < total);
}

#[test]
fn query_budget_is_shared_across_microtasks_promises_timers_and_listeners() {
    for setup in [
        "queueMicrotask(()=>{subject.matches('*'); afterCallback=true})",
        "Promise.resolve().then(()=>{subject.matches('*'); afterCallback=true})",
        "setTimeout(()=>{subject.matches('*'); afterCallback=true},0)",
        "subject.addEventListener('x',()=>{subject.matches('*'); afterCallback=true}); subject.dispatchEvent(new Event('x'))",
    ] {
        let (mut rt, _) = bounded_runtime(Some(0));
        let evaluated = rt.evaluate(&format!("var afterCallback=false; {setup}"));
        if setup.starts_with("setTimeout") {
            evaluated.unwrap();
            assert_eq!(rt.run_until_idle(), Err(Abort::SelectorWork));
        } else {
            assert_eq!(
                evaluated,
                Err(RuntimeError::Aborted(Abort::SelectorWork)),
                "{setup}"
            );
        }
        assert!(!global_bool(&mut rt, "afterCallback"));
        assert_eq!(
            rt.evaluate("true"),
            Err(RuntimeError::Aborted(Abort::SelectorWork))
        );
    }
}

#[test]
fn syntax_errors_remain_distinct_and_no_match_remains_a_valid_result() {
    let (mut zero, _) = bounded_runtime(Some(0));
    ok(
        &mut zero,
        "try {subject.matches('p['); false} catch(e) {e.name === 'SyntaxError'}",
    );
    assert_eq!(used(&mut zero), 0);
    let (mut rt, _) = bounded_runtime(None);
    ok(
        &mut rt,
        "!subject.matches('.missing') && subject.closest('.missing') === null && document.querySelector('.missing') === null && document.querySelectorAll('.missing').length === 0",
    );
    let before = used(&mut rt);
    ok(
        &mut rt,
        "var child=document.createElement('li'); subject.appendChild(child); var first=subject.querySelectorAll('li:nth-child(odd)'); subject.insertBefore(document.createElement('li'), child); first.length===1 && subject.querySelectorAll('li:nth-child(odd)').length===1 && child.matches(':nth-child(2)')",
    );
    assert!(used(&mut rt) > before);
}

pub(super) fn wide_runtime(n: usize, parents: usize, limit: Option<u64>) -> (DomRuntime, usize) {
    let (mut host, _, _, body) = StubHost::page();
    for _ in 0..parents {
        let parent = host.document.append_element(Some(body), "ul", None::<&str>);
        for index in 0..n {
            let child = host
                .document
                .append_element(Some(parent), "li", None::<&str>);
            host.document
                .set_element_attribute(
                    child,
                    "class",
                    if index % 3 == 0 { "visible" } else { "other" },
                )
                .unwrap();
        }
    }
    let mut rt = DomRuntime::with_options(
        host,
        RunOptions {
            limits: Limits {
                max_selector_work: limit,
                ..Limits::default()
            },
        },
    )
    .unwrap();
    expose(&mut rt, "subject", body);
    (rt, body)
}

#[test]
fn production_queries_stop_after_real_filtered_cache_limits() {
    for (n, parents, keys, limit) in [
        (65_537, 1, 1, 500_000),
        (32_768, 3, 1, 800_000),
        (256, 1, 257, 200_000),
    ] {
        let (mut rt, _) = wide_runtime(n, parents, Some(limit));
        let source = if keys == 1 {
            "li:nth-child(n of .visible)".to_owned()
        } else {
            (0..keys)
                .map(|i| format!("li:nth-child(n of .missing{i})"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let count = with_state(rt.context_mut(), |s| s.host.document().node_count()).unwrap();
        let result = rt.evaluate(&format!("var returned=false; var nodes=subject.querySelectorAll('{source}'); returned=true; subject.appendChild(document.createElement('b'))"));
        assert_eq!(
            result,
            Err(RuntimeError::Aborted(Abort::SelectorWork)),
            "n={n} parents={parents} keys={keys}"
        );
        assert!(!global_bool(&mut rt, "returned"));
        assert_eq!(used(&mut rt), limit);
        let after =
            with_state_unchecked(rt.context_mut(), |s| s.host.document().node_count()).unwrap();
        assert_eq!(after, count);
    }
}

#[test]
fn outer_traversal_is_bounded_even_without_element_candidates() {
    let (mut host, _, _, body) = StubHost::page();
    for _ in 0..1000 {
        host.document.append_comment(Some(body), "ignored");
    }
    let mut rt = DomRuntime::with_options(
        host,
        RunOptions {
            limits: Limits {
                max_selector_work: Some(20),
                ..Limits::default()
            },
        },
    )
    .unwrap();
    expose(&mut rt, "subject", body);
    assert_eq!(
        rt.evaluate("subject.querySelectorAll('*')"),
        Err(RuntimeError::Aborted(Abort::SelectorWork))
    );
    assert_eq!(used(&mut rt), 20);
}

#[test]
fn earlier_abort_reason_wins_over_later_query_attempts() {
    let (host, _, _, body) = StubHost::page();
    let mut rt = DomRuntime::with_options(
        host,
        RunOptions {
            limits: Limits {
                max_enqueued_jobs: 0,
                max_selector_work: Some(0),
                ..Limits::default()
            },
        },
    )
    .unwrap();
    expose(&mut rt, "subject", body);
    assert_eq!(
        rt.evaluate("setTimeout(()=>{},0); subject.matches('*')"),
        Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
    );
    assert_eq!(
        rt.evaluate("subject.matches('*')"),
        Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
    );
    assert_eq!(used(&mut rt), 0);
}
