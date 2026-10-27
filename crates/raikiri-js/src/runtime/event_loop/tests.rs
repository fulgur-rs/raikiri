use boa_engine::error::RuntimeLimitError;
use boa_engine::job::{GenericJob, IntervalJob, Job, NativeAsyncJob, NativeJob, TimeoutJob};
use boa_engine::{JsError, JsNativeError, JsValue, Source};

use super::{Abort, Limits, RunOptions, abort_for};
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state_unchecked as with_state;
use crate::runtime::{DomRuntime, RuntimeError};

fn rt() -> DomRuntime {
    runtime_with(Limits::default())
}

fn runtime_with(limits: Limits) -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::with_options(host, RunOptions { limits }).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

fn uncaught(rt: &mut DomRuntime) -> Vec<String> {
    with_state(rt.context_mut(), |s| s.event_loop.uncaught_errors.clone()).unwrap()
}

#[test]
fn limits_defaults() {
    let limits = Limits::default();
    assert_eq!(limits.max_virtual_time_ms, 30_000.0);
    assert_eq!(limits.max_tasks, 100_000);
    assert_eq!(limits.max_enqueued_jobs, 100_000);
    assert_eq!(limits.max_loop_iterations, 10_000_000);
    assert_eq!(limits.max_recursion, 512);
    assert_eq!(limits.max_nodes, 1_000_000);
    assert_eq!(RunOptions::default().limits.max_tasks, 100_000);
}

#[test]
fn microtasks_run_before_tasks() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log=[]; Promise.resolve().then(()=>log.push('a')); \
         setTimeout(()=>log.push('b'),0); queueMicrotask(()=>log.push('c')); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "log.join() === 'a,c,b'");
}

#[test]
fn evaluate_performs_a_microtask_checkpoint() {
    let mut rt = rt();
    rt.evaluate("var p = 0; Promise.resolve().then(()=>{ p = 1; });")
        .unwrap();
    ok(&mut rt, "p === 1");
    // The checkpoint also runs after a script that throws.
    let err = rt.evaluate("Promise.resolve().then(()=>{ p = 2; }); throw new Error('x')");
    assert!(matches!(err, Err(RuntimeError::JavaScript(_))), "{err:?}");
    ok(&mut rt, "p === 2");
}

#[test]
fn tasks_run_in_due_then_registration_order() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var o=[]; setTimeout(()=>o.push(2),20); setTimeout(()=>o.push(1),10); \
         setTimeout(()=>{o.push(3); Promise.resolve().then(()=>o.push('m'));},10); \
         setTimeout(()=>o.push(4),10); true",
    );
    rt.run_until_idle().unwrap();
    ok(
        &mut rt,
        "o.join() === '1,3,m,4,2' && performance.now() >= 20",
    );
    assert_eq!(rt.now(), 20.0);
}

#[test]
fn set_timeout_passes_arguments_and_returns_positive_ids() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var got; var a = setTimeout(function(x, y){ got = [this === window, x, y]; }, 0, 1, 2); \
         var b = setInterval(function(){}, 1); clearInterval(b); \
         a > 0 && b > a && Number.isInteger(a)",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "got.join() === 'true,1,2'");
}

#[test]
fn negative_and_nan_delays_are_zero() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var o=[]; setTimeout(()=>o.push('late'), 1); setTimeout(()=>o.push('neg'), -50); \
         setTimeout(()=>o.push('nan'), NaN); setTimeout(()=>o.push('none')); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "o.join() === 'neg,nan,none,late'");
}

#[test]
fn clear_timeout_cancels_and_shares_the_id_space() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var fired = []; var t = setTimeout(()=>fired.push('t'), 5); clearInterval(t); \
         var i = setInterval(()=>fired.push('i'), 5); clearTimeout(i); \
         clearTimeout(); clearTimeout(9999); clearTimeout('nope'); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "fired.length === 0");
    assert_eq!(rt.now(), 0.0);
}

#[test]
fn interval_repeats_until_cleared() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var n=0, id=setInterval(()=>{ if(++n===3) clearInterval(id); },5); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "n === 3");
    assert_eq!(rt.now(), 15.0);
}

#[test]
fn nested_timers_are_clamped_after_five_levels() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var times = []; var c = 0; \
         function f(){ times[++c] = performance.now(); if (c < 8) setTimeout(f, 0); } \
         setTimeout(f, 0); true",
    );
    rt.run_until_idle().unwrap();
    // The first six nesting levels run at time 0; the seventh and later
    // timers (nesting level > 5 when scheduled) wait at least 4ms.
    ok(
        &mut rt,
        "times[6] === 0 && times[7] === 4 && times[8] === 8",
    );
}

#[test]
fn interval_nesting_level_grows_with_each_repeat() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var ts = []; var id = setInterval(()=>{ ts.push(performance.now()); \
         if (ts.length === 8) clearInterval(id); }, 0); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "ts.join() === '0,0,0,0,0,0,4,8'");
}

#[test]
fn string_handlers_are_evaluated_in_the_global_scope() {
    let mut rt = rt();
    ok(&mut rt, "var s='x'; setTimeout('s = \"y\"', 0); true");
    rt.run_until_idle().unwrap();
    ok(&mut rt, "s === 'y'");
    // A non-function handler is stringified when the timer is set.
    ok(
        &mut rt,
        "var h = { toString(){ return 's = \"z\"'; } }; setTimeout(h); \
         h.toString = function(){ return 's = \"late\"'; }; true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "s === 'z'");
}

#[test]
fn string_handler_errors_are_recorded() {
    let mut rt = rt();
    rt.evaluate("setTimeout('throw new TypeError(\"bad\")'); setTimeout('(');")
        .unwrap();
    rt.run_until_idle().unwrap();
    let errors = uncaught(&mut rt);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].contains("TypeError: bad"), "{errors:?}");
    assert!(errors[1].contains("SyntaxError"), "{errors:?}");
}

#[test]
fn callback_exceptions_are_recorded_and_the_loop_continues() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var after = false; setTimeout(()=>{ throw new Error('one'); }, 0); \
         requestAnimationFrame(()=>{ throw new RangeError('two'); }); \
         queueMicrotask(()=>{ throw 'three'; }); \
         setTimeout(()=>{ after = true; }, 50); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "after");
    let errors = uncaught(&mut rt);
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors[0].contains("three"), "{errors:?}");
    assert!(errors[1].contains("Error: one"), "{errors:?}");
    assert!(errors[2].contains("RangeError: two"), "{errors:?}");
}

#[test]
fn animation_frames_share_a_timestamp_on_a_16ms_boundary() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var ts=[]; requestAnimationFrame(t=>ts.push(t)); requestAnimationFrame(t=>ts.push(t)); true",
    );
    rt.run_until_idle().unwrap();
    ok(
        &mut rt,
        "ts.length === 2 && ts[0] === ts[1] && ts[0] % 16 === 0",
    );
    ok(&mut rt, "ts[0] === 16");
}

#[test]
fn animation_frames_registered_during_a_frame_run_in_the_next_one() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var ts=[]; var cancelled = false; \
         requestAnimationFrame(t=>{ ts.push(t); requestAnimationFrame(u=>ts.push(u)); \
                                    cancelAnimationFrame(later); }); \
         var later = requestAnimationFrame(()=>{ cancelled = true; }); \
         setTimeout(()=>{}, 20); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "ts.join() === '16,32' && !cancelled");
}

#[test]
fn cancel_animation_frame_before_the_frame() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var ran = false; var id = requestAnimationFrame(()=>{ ran = true; }); \
         cancelAnimationFrame(id); cancelAnimationFrame(12345); cancelAnimationFrame(); \
         id > 0",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "!ran");
}

#[test]
fn non_callable_callbacks_are_type_errors() {
    let mut rt = rt();
    for src in [
        "requestAnimationFrame(1)",
        "requestAnimationFrame()",
        "queueMicrotask('x')",
        "queueMicrotask()",
    ] {
        let err = rt.evaluate(src);
        assert!(
            matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
            "{src}: {err:?}"
        );
    }
}

#[test]
fn performance_now_is_virtual_time() {
    let mut rt = rt();
    ok(&mut rt, "performance.now() === 0");
    assert_eq!(rt.now(), 0.0);
    rt.evaluate("setTimeout(function(){}, 1234)").unwrap();
    rt.run_until_idle().unwrap();
    assert_eq!(rt.now(), 1234.0);
    ok(&mut rt, "performance.now() === 1234");
}

#[test]
fn run_until_idle_on_an_empty_queue_is_ok() {
    let mut rt = rt();
    assert_eq!(rt.run_until_idle(), Ok(()));
}

#[test]
fn task_limit_aborts_and_discards_the_queue() {
    let mut rt = runtime_with(Limits {
        max_tasks: 50,
        ..Default::default()
    });
    rt.evaluate("var ticks = 0; setInterval(function(){ ticks++; }, 0)")
        .unwrap();
    assert_eq!(rt.run_until_idle(), Err(Abort::Tasks));
    let ticks = rt.context_mut().eval(Source::from_bytes("ticks")).unwrap();
    assert_eq!(ticks, JsValue::from(50));
    // Once aborted the runtime refuses to run anything else.
    assert_eq!(rt.run_until_idle(), Err(Abort::Tasks));
    let err = rt.evaluate("1");
    assert_eq!(err, Err(RuntimeError::Aborted(Abort::Tasks)));
}

#[test]
fn virtual_time_limit_aborts() {
    let mut rt = runtime_with(Limits {
        max_virtual_time_ms: 1000.0,
        ..Default::default()
    });
    rt.evaluate("var ran = false; setTimeout(function(){ ran = true; }, 5000)")
        .unwrap();
    assert_eq!(rt.run_until_idle(), Err(Abort::VirtualTime));
    assert!(rt.now() <= 1000.0);
}

#[test]
fn unbounded_microtask_chains_hit_the_task_limit() {
    let mut rt = runtime_with(Limits {
        max_tasks: 100,
        ..Default::default()
    });
    let err = rt.evaluate("function f(){ queueMicrotask(f); } f();");
    assert_eq!(err, Err(RuntimeError::Aborted(Abort::Tasks)));
}

fn loop_limited() -> DomRuntime {
    runtime_with(Limits {
        max_loop_iterations: 10_000,
        ..Default::default()
    })
}

#[test]
fn loop_limit_is_not_catchable_by_script() {
    for src in [
        "try { while (true) {} } catch (e) { 'caught' }",
        "try { while (true) {} } finally { 'finally' }",
        "try { [1].forEach(()=>{ while (true) {} }); } catch (e) { 'caught' }",
        "new Promise(()=>{ while (true) {} }).catch(()=>{}); 'caught'",
        "async function f(){ while (true) {} } f().catch(()=>{}); 'caught'",
        "var p = Promise.resolve().then(()=>{ while (true) {} }).catch(()=>{}); 'caught'",
    ] {
        let mut rt = loop_limited();
        let result = rt.evaluate(src);
        assert_eq!(
            result,
            Err(RuntimeError::Aborted(Abort::LoopIterations)),
            "{src}"
        );
        assert_eq!(rt.run_until_idle(), Err(Abort::LoopIterations), "{src}");
    }
}

#[test]
fn recursion_limit_is_not_catchable_by_script() {
    let mut rt = runtime_with(Limits {
        max_recursion: 64,
        ..Default::default()
    });
    let result = rt.evaluate("function f(){ f(); } try { f() } catch (e) { 'caught' }");
    assert_eq!(result, Err(RuntimeError::Aborted(Abort::Recursion)));
}

#[test]
fn limits_inside_callbacks_abort_the_loop() {
    for src in [
        "setTimeout(()=>{ while (true) {} }, 0)",
        "setTimeout('while (true) {}', 0)",
        "requestAnimationFrame(()=>{ while (true) {} })",
        "setTimeout(()=>{ Promise.resolve().then(()=>{ while (true) {} }); }, 0)",
        "setTimeout(()=>{ queueMicrotask(()=>{ while (true) {} }); }, 0)",
    ] {
        let mut rt = loop_limited();
        rt.evaluate(&format!(
            "var later = false; setTimeout(()=>{{ later = true; }}, 100); {src}"
        ))
        .unwrap();
        assert_eq!(rt.run_until_idle(), Err(Abort::LoopIterations), "{src}");
        assert!(uncaught(&mut rt).is_empty(), "{src}");
        // The queue was discarded: the later timer never ran.
        let later = with_state(rt.context_mut(), |s| s.event_loop.tasks.len()).unwrap();
        assert_eq!(later, 0, "{src}");
    }
}

/// Deeply nested source parses even on a thread whose own stack is far
/// smaller than Boa's recursive-descent parser needs for it: as a classic
/// script, as a string timer, and through `eval` in a function timer and a
/// promise reaction.
#[test]
fn deeply_nested_source_does_not_overflow_a_small_thread_stack() {
    const DEPTH: usize = 40;
    let nested = format!("{}1{}", "[".repeat(DEPTH), "]".repeat(DEPTH));
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let mut rt = rt();
            ok(&mut rt, &format!("Array.isArray({nested})"));
            rt.evaluate(&format!(
                "var src = '{nested}'; var results = [];
                 Promise.resolve().then(() => results.push(Array.isArray(eval(src))));
                 setTimeout(() => results.push(Array.isArray(eval(src))), 0);
                 setTimeout('results.push(Array.isArray({nested}))', 0);"
            ))
            .unwrap();
            rt.run_until_idle().unwrap();
            ok(&mut rt, "results.length === 3 && results.every(r => r)");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn realm_limits_are_applied() {
    let mut rt = runtime_with(Limits {
        max_loop_iterations: 123,
        max_recursion: 45,
        ..Default::default()
    });
    let limits = rt.context_mut().runtime_limits();
    assert_eq!(limits.loop_iteration_limit(), 123);
    assert_eq!(limits.recursion_limit(), 45);
}

#[test]
fn engine_errors_map_to_aborts() {
    let loop_error = JsError::from(RuntimeLimitError::LoopIteration);
    let recursion = JsError::from(RuntimeLimitError::Recursion);
    let stack = JsError::from(RuntimeLimitError::StackSize);
    let native = JsError::from(JsNativeError::typ());
    let opaque = JsError::from_opaque(JsValue::from(1));
    assert_eq!(abort_for(&loop_error), Some(Abort::LoopIterations));
    assert_eq!(abort_for(&recursion), Some(Abort::Recursion));
    assert_eq!(abort_for(&stack), Some(Abort::Recursion));
    assert_eq!(abort_for(&native), None);
    assert_eq!(abort_for(&opaque), None);
}

#[test]
fn abort_display() {
    for (abort, text) in [
        (Abort::VirtualTime, "virtual time limit"),
        (Abort::Tasks, "task limit"),
        (Abort::EnqueuedJobs, "enqueued job limit"),
        (Abort::LoopIterations, "loop iteration limit"),
        (Abort::Recursion, "recursion limit"),
        (Abort::Nodes, "node limit"),
    ] {
        assert!(abort.to_string().contains(text), "{abort:?}");
        assert!(
            RuntimeError::Aborted(abort.clone())
                .to_string()
                .contains(text)
        );
    }
}

#[test]
fn boa_native_jobs_run_on_the_virtual_clock() {
    let mut rt = rt();
    rt.evaluate("var log = [];").unwrap();
    let context = rt.context_mut();
    context.enqueue_job(Job::TimeoutJob(TimeoutJob::new(
        NativeJob::new(|ctx| ctx.eval(Source::from_bytes("log.push('timeout')"))),
        30,
    )));
    let interval_runs = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let runs = interval_runs.clone();
    let interval = IntervalJob::new(
        boa_engine::job::NativeJobFn::new(move |_| {
            runs.set(runs.get() + 1);
            Ok(JsValue::undefined())
        }),
        10,
    );
    let token = interval.cancellation_token().clone();
    context.enqueue_job(Job::IntervalJob(interval));
    let realm = context.realm().clone();
    context.enqueue_job(Job::GenericJob(GenericJob::new(
        |ctx| ctx.eval(Source::from_bytes("log.push('generic')")),
        realm,
    )));
    context.enqueue_job(Job::AsyncJob(NativeAsyncJob::new(async |ctx| {
        ctx.borrow_mut()
            .eval(Source::from_bytes("log.push('async')"))
    })));
    context.enqueue_job(Job::FinalizationRegistryCleanupJob(NativeAsyncJob::new(
        async |_| Ok(JsValue::undefined()),
    )));
    rt.evaluate("setTimeout(()=>log.push('js'), 25)").unwrap();
    ok(&mut rt, "log.join() === 'generic,async'");
    // Stop the interval from inside a later timer so the loop can go idle.
    let context = rt.context_mut();
    context.enqueue_job(Job::TimeoutJob(TimeoutJob::new(
        NativeJob::new(move |ctx| {
            token.cancel(ctx);
            Ok(JsValue::undefined())
        }),
        35,
    )));
    rt.run_until_idle().unwrap();
    ok(&mut rt, "log.join() === 'generic,async,js,timeout'");
    assert_eq!(interval_runs.get(), 3);
}

#[test]
fn boa_job_errors_are_recorded() {
    let mut rt = rt();
    let context = rt.context_mut();
    context.enqueue_job(Job::TimeoutJob(TimeoutJob::new(
        NativeJob::new(|_| Err(JsNativeError::typ().with_message("native").into())),
        0,
    )));
    rt.run_until_idle().unwrap();
    let errors = uncaught(&mut rt);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("TypeError: native"), "{errors:?}");
}

#[test]
fn run_jobs_is_a_microtask_checkpoint() {
    let mut rt = rt();
    rt.context_mut()
        .eval(Source::from_bytes(
            "var q = 0; Promise.resolve().then(()=>{ q = 1; });",
        ))
        .unwrap();
    rt.context_mut().run_jobs().unwrap();
    ok(&mut rt, "q === 1");
}

#[test]
fn run_jobs_reports_a_limit_as_an_error() {
    let mut rt = runtime_with(Limits {
        max_tasks: 3,
        ..Default::default()
    });
    rt.context_mut()
        .eval(Source::from_bytes(
            "for (var i = 0; i < 5; i++) queueMicrotask(function(){});",
        ))
        .unwrap();
    let err = rt.context_mut().run_jobs().unwrap_err();
    assert!(err.to_string().contains("task limit"), "{err}");
    assert_eq!(rt.run_until_idle(), Err(Abort::Tasks));
}

#[test]
fn pending_async_jobs_are_dropped_after_one_poll() {
    let mut rt = rt();
    rt.context_mut()
        .enqueue_job(Job::AsyncJob(NativeAsyncJob::new(async |_| {
            std::future::pending::<()>().await;
            Ok(JsValue::undefined())
        })));
    assert_eq!(rt.run_until_idle(), Ok(()));
}

#[test]
fn queued_tasks_order_by_due_time_then_registration() {
    let queued = |due: f64, seq: u64| super::Queued {
        due,
        seq,
        task: super::Task::AnimationFrame,
    };
    assert!(queued(1.0, 0) == queued(1.0, 0));
    assert!(queued(1.0, 0) != queued(1.0, 1));
    // Reversed for the max-heap: earlier sorts greater.
    assert!(queued(1.0, 5) > queued(2.0, 0));
    assert!(queued(1.0, 0) > queued(1.0, 1));
}

#[test]
fn microtasks_run_after_each_animation_frame_callback() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log = []; \
         requestAnimationFrame(()=>{ Promise.resolve().then(()=>log.push('m')); }); \
         requestAnimationFrame(()=>log.push('b')); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "log.join() === 'm,b'");
}

#[test]
fn microtasks_after_a_timer_callback_keep_its_nesting_level() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var ts = []; \
         function f(){ ts.push(performance.now()); \
                       if (ts.length < 8) Promise.resolve().then(()=>setTimeout(f, 0)); } \
         setTimeout(f, 0); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "ts.join() === '0,0,0,0,0,0,4,8'");
}

#[test]
fn cancelled_boa_timeout_jobs_do_not_run() {
    let mut rt = rt();
    let ran = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = ran.clone();
    let job = TimeoutJob::new(
        NativeJob::new(move |_| {
            flag.set(true);
            Ok(JsValue::undefined())
        }),
        5,
    );
    let token = job.cancellation_token().clone();
    let context = rt.context_mut();
    context.enqueue_job(Job::TimeoutJob(job));
    token.cancel(context);
    rt.run_until_idle().unwrap();
    assert!(!ran.get());
}

#[test]
fn timers_without_a_handler_are_type_errors() {
    let mut rt = rt();
    for src in ["setTimeout()", "setInterval()"] {
        let err = rt.evaluate(src);
        assert!(
            matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
            "{src}: {err:?}"
        );
    }
    assert_eq!(rt.run_until_idle(), Ok(()));
    assert_eq!(rt.now(), 0.0);
}

#[test]
fn into_host_after_an_abort_returns_the_document() {
    let mut rt = runtime_with(Limits {
        max_tasks: 5,
        ..Default::default()
    });
    rt.evaluate(
        "document.body.appendChild(document.createElement('p')); \
         setInterval(function(){}, 0);",
    )
    .unwrap();
    assert_eq!(rt.run_until_idle(), Err(Abort::Tasks));
    let host = rt.into_host();
    let document = host.document();
    let p = (0..document.node_count())
        .filter_map(|i| document.get_node(i))
        .filter(|n| n.kind() == raikiri_dom::NodeKind::Element)
        .count();
    assert_eq!(p, 4, "html, head, body, p");
}

#[test]
fn enqueue_budget_stops_bursts_and_cancelled_timer_churn() {
    for source in [
        "for(let i=0;i<4;i++) queueMicrotask(()=>{});",
        "for(let i=0;i<4;i++) clearTimeout(setTimeout(()=>{},0));",
        "for(let i=0;i<4;i++) requestAnimationFrame(()=>{});",
        "for(let i=0;i<4;i++) Promise.resolve().then(()=>{});",
    ] {
        let mut rt = runtime_with(Limits {
            max_enqueued_jobs: 3,
            ..Limits::default()
        });
        assert_eq!(
            rt.evaluate(source),
            Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
        );
        assert_eq!(rt.run_until_idle(), Err(Abort::EnqueuedJobs));
        assert_eq!(
            rt.evaluate("true"),
            Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
        );
        with_state(rt.context_mut(), |state| {
            let event_loop = &state.event_loop;
            assert!(event_loop.tasks.is_empty());
            assert!(event_loop.microtasks.is_empty());
            assert!(event_loop.timers.is_empty());
            assert!(event_loop.frame_callbacks.is_empty());
            assert!(event_loop.cancellable.is_empty());
            assert_eq!(event_loop.tasks.capacity(), 0);
            assert_eq!(event_loop.microtasks.capacity(), 0);
            assert_eq!(event_loop.timers.capacity(), 0);
            assert_eq!(event_loop.cancellable.capacity(), 0);
            assert_eq!(event_loop.jobs_run, 0);
        })
        .unwrap();
    }
}

#[test]
fn enqueue_budget_exact_zero_and_execution_limits_are_independent() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 3,
        max_tasks: 1,
        ..Limits::default()
    });
    rt.evaluate("requestAnimationFrame(()=>{}); requestAnimationFrame(()=>{}); requestAnimationFrame(()=>{});").unwrap();
    assert_eq!(rt.run_until_idle(), Ok(()));
    with_state(rt.context_mut(), |s| {
        assert_eq!(s.event_loop.jobs_run, 1);
        assert_eq!(s.event_loop.jobs_enqueued, 3);
    })
    .unwrap();
    assert_eq!(
        rt.evaluate("setTimeout(()=>{},0)"),
        Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
    );
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 0,
        ..Limits::default()
    });
    ok(&mut rt, "1+1===2");
    assert_eq!(
        rt.evaluate("queueMicrotask(()=>{})"),
        Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
    );
}

#[test]
fn enqueue_budget_blocks_dom_actions_after_promise_overflow() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 1,
        ..Limits::default()
    });
    let before = with_state(rt.context_mut(), |s| s.host.document().node_count()).unwrap();
    assert_eq!(rt.evaluate("var survived=false; Promise.resolve().then(()=>{}); Promise.resolve().then(()=>{}); survived=true; document.body.appendChild(document.createElement('p'));"), Err(RuntimeError::Aborted(Abort::EnqueuedJobs)));
    with_state(rt.context_mut(), |s| {
        assert_eq!(s.host.document().node_count(), before)
    })
    .unwrap();
    // Boa's enqueue hook cannot throw: ordinary JS between overflow and the
    // next host boundary still executes, but no queued callback does.
    assert!(
        rt.context_mut()
            .global_object()
            .get(boa_engine::js_string!("survived"), rt.context_mut())
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn enqueue_budget_applies_to_interval_rescheduling_and_recursive_microtasks() {
    for source in [
        "setInterval(()=>{},0);",
        "queueMicrotask(function f(){queueMicrotask(f)});",
    ] {
        let mut rt = runtime_with(Limits {
            max_enqueued_jobs: 4,
            ..Limits::default()
        });
        let result = rt.evaluate(source);
        if result.is_ok() {
            assert_eq!(rt.run_until_idle(), Err(Abort::EnqueuedJobs));
        } else {
            assert_eq!(result, Err(RuntimeError::Aborted(Abort::EnqueuedJobs)));
        }
        with_state(rt.context_mut(), |s| {
            assert_eq!(s.event_loop.jobs_enqueued, 4)
        })
        .unwrap();
    }
}

#[test]
fn enqueue_budget_counts_boa_and_native_registration_routes() {
    for kind in 0..5 {
        let mut rt = runtime_with(Limits {
            max_enqueued_jobs: 0,
            ..Limits::default()
        });
        let context = rt.context_mut();
        match kind {
            0 => context.enqueue_job(Job::GenericJob(GenericJob::new(
                |_| Ok(JsValue::undefined()),
                context.realm().clone(),
            ))),
            1 => context.enqueue_job(Job::AsyncJob(NativeAsyncJob::new(async |_| {
                Ok(JsValue::undefined())
            }))),
            2 => context.enqueue_job(Job::TimeoutJob(TimeoutJob::new(
                NativeJob::new(|_| Ok(JsValue::undefined())),
                0,
            ))),
            3 => context.enqueue_job(Job::IntervalJob(IntervalJob::new(
                boa_engine::job::NativeJobFn::new(|_| Ok(JsValue::undefined())),
                0,
            ))),
            _ => assert!(
                super::queue_native_task(context, NativeJob::new(|_| Ok(JsValue::undefined())))
                    .is_err()
            ),
        }
        assert_eq!(rt.run_until_idle(), Err(Abort::EnqueuedJobs));
        // An executor hook invoked after abort must not retain its job.
        rt.context_mut()
            .enqueue_job(Job::AsyncJob(NativeAsyncJob::new(async |_| {
                panic!("aborted job ran")
            })));
        with_state(rt.context_mut(), |s| {
            assert!(s.event_loop.tasks.is_empty());
            assert!(s.event_loop.microtasks.is_empty());
            assert_eq!(s.event_loop.jobs_enqueued, 0);
        })
        .unwrap();
    }
}

#[test]
fn enqueue_budget_drops_cancellable_jobs_and_preserves_first_reason() {
    let mut event_loop = super::EventLoop::new(Limits {
        max_enqueued_jobs: 2,
        ..Limits::default()
    });
    let retained = std::rc::Rc::new(());
    let capture = retained.clone();
    event_loop
        .queue_cancellable_native_task(
            10.0,
            NativeJob::new(move |_| {
                let _ = &capture;
                Ok(JsValue::undefined())
            }),
        )
        .unwrap();
    let token = event_loop
        .queue_cancellable_native_task(20.0, NativeJob::new(|_| Ok(JsValue::undefined())))
        .unwrap();
    event_loop.cancel_native_task(token);
    assert_eq!(event_loop.jobs_enqueued, 2);
    assert!(
        event_loop
            .queue_cancellable_native_task(0.0, NativeJob::new(|_| Ok(JsValue::undefined())))
            .is_err()
    );
    assert_eq!(std::rc::Rc::strong_count(&retained), 1);
    assert!(event_loop.cancellable.is_empty());
    assert!(event_loop.tasks.is_empty());
    assert_eq!(event_loop.tasks.capacity(), 0);
    assert_eq!(event_loop.microtasks.capacity(), 0);
    assert_eq!(event_loop.cancellable.capacity(), 0);
    event_loop.abort(Abort::Tasks);
    assert_eq!(event_loop.aborted, Some(Abort::EnqueuedJobs));
    assert_eq!(
        event_loop.queue_task(0.0, super::Task::AnimationFrame),
        Err(Abort::EnqueuedJobs)
    );
}

#[test]
fn promise_only_overflow_waits_for_engine_limit_but_never_retains_more_jobs() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 2,
        max_loop_iterations: 20,
        ..Limits::default()
    });
    assert_eq!(
        rt.evaluate("while(true) Promise.resolve().then(()=>{});"),
        Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
    );
    with_state(rt.context_mut(), |s| {
        assert_eq!(s.event_loop.jobs_enqueued, 2);
        assert!(s.event_loop.microtasks.is_empty());
    })
    .unwrap();
}

#[test]
fn enqueue_overflow_is_uncatchable_and_later_reports_do_not_repeat_console() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 0,
        ..Limits::default()
    });
    assert_eq!(rt.evaluate("var caught=false, finalized=false; console.log('before'); try {setTimeout(()=>{},0)} catch(e){caught=true} finally {finalized=true}"), Err(RuntimeError::Aborted(Abort::EnqueuedJobs)));
    let context = rt.context_mut();
    let global = context.global_object();
    assert!(
        !global
            .get(boa_engine::js_string!("caught"), context)
            .unwrap()
            .to_boolean()
    );
    assert!(
        !global
            .get(boa_engine::js_string!("finalized"), context)
            .unwrap()
            .to_boolean()
    );
    let report = rt.run_document();
    assert_eq!(report.aborted, Some(Abort::EnqueuedJobs));
    assert!(report.console.is_empty());
}

#[test]
#[ignore = "manual resource measurement; select workload/count using RAIKIRI_QUEUE_PROBE_* env vars"]
fn enqueue_resource_probe() {
    let workload = std::env::var("RAIKIRI_QUEUE_PROBE_KIND").unwrap_or_else(|_| "churn".into());
    let count: usize = std::env::var("RAIKIRI_QUEUE_PROBE_COUNT")
        .unwrap_or_else(|_| "300000".into())
        .parse()
        .unwrap();
    let operation = match workload.as_str() {
        "churn" => "clearTimeout(setTimeout(callback, 0));",
        "timer" => "setTimeout(callback, 0);",
        "microtask" => "queueMicrotask(callback);",
        "promise" => "Promise.resolve().then(callback);",
        "frame" => "requestAnimationFrame(callback);",
        _ => panic!("unknown probe workload"),
    };
    let mut rt = rt();
    rt.context_mut()
        .eval(Source::from_bytes("var callback=()=>{};"))
        .unwrap();
    let start = std::time::Instant::now();
    let mut high_water = 0;
    let mut completed = 0;
    let mut abort = None;
    for offset in (0..count).step_by(1000) {
        let batch = (count - offset).min(1000);
        let source = format!("for(var i=0;i<{batch};i++){{{operation}}}");
        let result = rt.context_mut().eval(Source::from_bytes(&source));
        let (pending, reason) = with_state(rt.context_mut(), |s| {
            (
                s.event_loop.tasks.len()
                    + s.event_loop.microtasks.len()
                    + s.event_loop.frame_callbacks.len(),
                s.event_loop.aborted.clone(),
            )
        })
        .unwrap();
        high_water = high_water.max(pending);
        abort = reason;
        if result.is_err() || abort.is_some() {
            break;
        }
        completed += batch;
    }
    let elapsed = start.elapsed();
    let (tasks, microtasks, timers, frames) = with_state(rt.context_mut(), |s| {
        (
            s.event_loop.tasks.len(),
            s.event_loop.microtasks.len(),
            s.event_loop.timers.len(),
            s.event_loop.frame_callbacks.len(),
        )
    })
    .unwrap();
    let memory = std::fs::read_to_string("/proc/self/status").unwrap();
    let peak = memory
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .unwrap();
    println!(
        "QUEUE_PROBE kind={workload} requested={count} completed={completed} elapsed_ms={:.3} high_water={high_water} tasks={tasks} microtasks={microtasks} timers={timers} frames={frames} abort={abort:?} {peak}",
        elapsed.as_secs_f64() * 1000.0
    );
}

#[test]
fn animation_finish_jobs_obey_enqueue_budget() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 0,
        ..Limits::default()
    });
    assert_eq!(
        rt.evaluate("document.body.animate({opacity:[0,1]}, 20).finished;"),
        Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
    );
    assert_eq!(rt.run_until_idle(), Err(Abort::EnqueuedJobs));
    with_state(rt.context_mut(), |s| {
        assert!(s.event_loop.cancellable.is_empty())
    })
    .unwrap();
}

#[test]
fn effectively_unlimited_enqueue_budget_keeps_cancelled_task_execution_semantics() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: u64::MAX,
        max_tasks: 0,
        ..Limits::default()
    });
    rt.evaluate("for(let i=0;i<1000;i++) clearTimeout(setTimeout(()=>{},0));")
        .unwrap();
    assert_eq!(rt.run_until_idle(), Ok(()));
    with_state(rt.context_mut(), |s| {
        assert_eq!(s.event_loop.jobs_enqueued, 1000);
        assert_eq!(s.event_loop.jobs_run, 0);
    })
    .unwrap();
}

#[test]
fn silent_enqueue_abort_blocks_public_native_callbacks() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 0,
        ..Limits::default()
    });
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = calls.clone();
    let result = rt.evaluate_with_callback(
        "Promise.resolve().then(()=>{}); notify();",
        "notify",
        move |_, _, _| {
            seen.set(seen.get() + 1);
            Ok(JsValue::undefined())
        },
    );
    assert_eq!(calls.get(), 0);
    assert_eq!(result, Err(RuntimeError::Aborted(Abort::EnqueuedJobs)));
}

#[test]
fn silent_enqueue_abort_during_error_conversion_is_reported_immediately() {
    for source in [
        "throw {toString(){Promise.resolve().then(()=>{}); return 'x'}};",
        "var e=new Error('x'); Object.defineProperty(e,'message',{get(){Promise.resolve().then(()=>{}); return 'x'}}); throw e;",
    ] {
        let mut rt = runtime_with(Limits {
            max_enqueued_jobs: 0,
            ..Limits::default()
        });
        assert_eq!(
            rt.evaluate(source),
            Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
        );
    }
}

#[test]
fn silent_enqueue_abort_stops_animation_iterator_callbacks() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 0,
        ..Limits::default()
    });
    assert_eq!(rt.evaluate("var nextCalls=0; document.body.animate({[Symbol.iterator](){Promise.resolve().then(()=>{});return{next(){nextCalls++;return{done:true}}}}},20);"), Err(RuntimeError::Aborted(Abort::EnqueuedJobs)));
    let context = rt.context_mut();
    assert_eq!(
        context
            .global_object()
            .get(boa_engine::js_string!("nextCalls"), context)
            .unwrap()
            .as_number(),
        Some(0.0)
    );
}

#[test]
fn silent_enqueue_abort_blocks_constructor_argument_callbacks() {
    for constructor in ["DOMRect", "URL", "URLSearchParams"] {
        let mut rt = runtime_with(Limits {
            max_enqueued_jobs: 0,
            ..Limits::default()
        });
        let source = format!(
            "var converted=false; Promise.resolve().then(()=>{{}}); new {constructor}({{valueOf(){{converted=true;return 1}},toString(){{converted=true;return 'https://example.test'}}}});"
        );
        assert_eq!(
            rt.evaluate(&source),
            Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
        );
        let context = rt.context_mut();
        assert!(
            !context
                .global_object()
                .get(boa_engine::js_string!("converted"), context)
                .unwrap()
                .to_boolean()
        );
    }
}

#[test]
fn silent_enqueue_abort_during_native_error_getters_is_reported_immediately() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 0,
        ..Limits::default()
    });
    assert_eq!(rt.evaluate("var e=new Error('x'); Object.defineProperty(e,'message',{get(){Promise.resolve().then(()=>{});return 'x'}}); throw e;"), Err(RuntimeError::Aborted(Abort::EnqueuedJobs)));
}

#[test]
fn silent_enqueue_abort_stops_later_constructor_conversions() {
    for source in [
        "new DOMRect({valueOf(){Promise.resolve().then(()=>{});return 1}},{valueOf(){later=true;return 2}});",
        "new URL({toString(){Promise.resolve().then(()=>{});return 'https://example.test'}},{toString(){later=true;return 'https://base.test'}});",
    ] {
        let mut rt = runtime_with(Limits {
            max_enqueued_jobs: 0,
            ..Limits::default()
        });
        rt.evaluate("var later=false;").unwrap();
        assert_eq!(
            rt.evaluate(source),
            Err(RuntimeError::Aborted(Abort::EnqueuedJobs))
        );
        let context = rt.context_mut();
        assert!(
            !context
                .global_object()
                .get(boa_engine::js_string!("later"), context)
                .unwrap()
                .to_boolean()
        );
    }
}

#[test]
fn silent_enqueue_abort_in_iterator_getter_blocks_iterator_method() {
    let mut rt = runtime_with(Limits {
        max_enqueued_jobs: 0,
        ..Limits::default()
    });
    assert_eq!(rt.evaluate("var iteratorCalls=0; document.body.animate({get [Symbol.iterator](){Promise.resolve().then(()=>{});return function(){iteratorCalls++;return{next(){return{done:true}}}}}},20);"), Err(RuntimeError::Aborted(Abort::EnqueuedJobs)));
    let context = rt.context_mut();
    assert_eq!(
        context
            .global_object()
            .get(boa_engine::js_string!("iteratorCalls"), context)
            .unwrap()
            .as_number(),
        Some(0.0)
    );
}
