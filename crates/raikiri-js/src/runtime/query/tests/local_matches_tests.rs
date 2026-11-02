use super::*;
use crate::runtime::webidl::with_state_unchecked;
use crate::runtime::{Abort, Limits, RunOptions, RuntimeError};

fn deep_runtime(depth: usize, limit: Option<u64>) -> DomRuntime {
    let (mut host, _, _, body) = StubHost::page();
    let mut leaf = body;
    for _ in 0..depth {
        leaf = host
            .document
            .append_element(Some(leaf), "div", None::<&str>);
    }
    host.document
        .set_element_attribute(leaf, "class", "probe")
        .unwrap();
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
    expose(&mut rt, "subject", leaf);
    rt
}

#[test]
fn local_matching_work_is_independent_of_depth_and_still_terminal() {
    let script = "subject.matches('*')&&subject.matches('div')&&subject.matches('.probe')&&subject.matches('[class]')";
    let mut counts = Vec::new();
    for depth in [1, 100, 8000] {
        let mut rt = deep_runtime(depth, None);
        ok(&mut rt, script);
        counts.push(
            with_state_unchecked(rt.context_mut(), |s| s.event_loop.selector_work.count()).unwrap(),
        );
    }
    assert!(counts[0] > 0);
    assert_eq!(counts, vec![counts[0]; 3]);
    let mut exact = deep_runtime(8000, Some(counts[0]));
    ok(&mut exact, script);
    assert_eq!(
        exact.evaluate("subject.matches('*')"),
        Err(RuntimeError::Aborted(Abort::SelectorWork))
    );
    assert_eq!(
        exact.evaluate("true"),
        Err(RuntimeError::Aborted(Abort::SelectorWork))
    );
}

#[test]
fn fallback_matching_keeps_real_context_after_mutation_and_detachment() {
    let mut rt = rt();
    rt.evaluate("var parent=document.createElement('section'),leaf=document.createElement('span');parent.setAttribute('lang','ja');parent.setAttribute('dir','rtl');parent.appendChild(leaf);document.body.appendChild(parent);").unwrap();
    let script = "leaf.matches('section > span')&&leaf.matches(':is(section > span)')&&leaf.matches(':not(body > span)')&&leaf.matches(':lang(ja)')&&leaf.matches(':dir(rtl)')&&leaf.matches(':scope:first-child')&&parent.matches(':has(> span)')&&!leaf.matches(':root')";
    ok(&mut rt, script);
    rt.evaluate("parent.remove();").unwrap();
    ok(&mut rt, script);
    ok(
        &mut rt,
        "leaf.matches({toString(){leaf.setAttribute('class','updated');return '.updated';}})",
    );
}
