use boa_engine::error::RuntimeLimitError;
use boa_engine::job::{GenericJob, IntervalJob, Job, NativeAsyncJob, NativeJob, TimeoutJob};
use boa_engine::{JsError, JsNativeError, JsValue, Source};

use super::{Abort, Limits, RunOptions, abort_for};
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;
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
            Err(RuntimeError::Aborted(Abort::LoopIteration)),
            "{src}"
        );
        assert_eq!(rt.run_until_idle(), Err(Abort::LoopIteration), "{src}");
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
        assert_eq!(rt.run_until_idle(), Err(Abort::LoopIteration), "{src}");
        assert!(uncaught(&mut rt).is_empty(), "{src}");
        // The queue was discarded: the later timer never ran.
        let later = with_state(rt.context_mut(), |s| s.event_loop.tasks.len()).unwrap();
        assert_eq!(later, 0, "{src}");
    }
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
    assert_eq!(abort_for(&loop_error), Some(Abort::LoopIteration));
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
        (Abort::LoopIteration, "loop iteration limit"),
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
