use boa_engine::{JsError, JsString, JsValue};

use crate::runtime::dispatch;
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::{node_index, with_state};
use crate::runtime::{Abort, DomRuntime, Limits, RunOptions, RuntimeError};

fn rt() -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

fn body_index(rt: &mut DomRuntime) -> usize {
    let value = rt.evaluate("document.body").unwrap();
    node_index(&value).unwrap()
}

fn uncaught(rt: &mut DomRuntime) -> Vec<String> {
    with_state(rt.context_mut(), |s| s.event_loop.uncaught_errors.clone()).unwrap()
}

// ---- DOM §2.9 dispatch order (capture / target / bubble) -----------------

#[test]
fn capture_then_target_registration_order_then_bubble() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log=[]; var b=document.body; var p=document.createElement('p'); b.appendChild(p);
  window.addEventListener('x', e=>log.push('wc'), true); document.addEventListener('x', e=>log.push('dc'), true);
  b.addEventListener('x', e=>log.push('bc'), true); p.addEventListener('x', e=>log.push('pt'));
  p.addEventListener('x', e=>log.push('ptc'), true); b.addEventListener('x', e=>log.push('bb'));
  window.addEventListener('x', e=>log.push('wb'));
  p.dispatchEvent(new Event('x', {bubbles:true})) && log.join() === 'wc,dc,bc,pt,ptc,bb,wb'",
    );
}

#[test]
fn passive_listener_preventdefault_is_ignored() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var e=new Event('y',{cancelable:true}); document.body.addEventListener('y', ev=>ev.preventDefault(), {passive:true}); document.body.dispatchEvent(e) === true && !e.defaultPrevented",
    );
}

#[test]
fn once_listener_removed_before_its_own_call_never_fires_again() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var c=0; document.body.addEventListener('z', ()=>c++, {once:true}); document.body.dispatchEvent(new Event('z')); document.body.dispatchEvent(new Event('z')); c === 1",
    );
}

#[test]
fn window_onerror_handler_gets_message_and_uncaught_errors_stays_empty_when_it_returns_true() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var seen=[]; window.onerror=function(m){ seen.push(m); return true; };
  document.body.addEventListener('w', ()=>{ throw new Error('boom'); }); document.body.addEventListener('w', ()=>seen.push('next'));
  document.body.dispatchEvent(new Event('w')); seen.length === 2 && /boom/.test(seen[0]) && seen[1] === 'next'",
    );
    assert!(uncaught(&mut rt).is_empty());
}

#[test]
fn window_onerror_handler_returning_a_falsy_value_leaves_the_exception_unhandled() {
    let mut rt = rt();
    ok(
        &mut rt,
        "window.onerror = function(){ return false; }; \
         document.body.addEventListener('w2', ()=>{ throw new Error('boom2'); }); \
         document.body.dispatchEvent(new Event('w2')); true",
    );
    let errors = uncaught(&mut rt);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("boom2"), "{errors:?}");
}

#[test]
fn custom_event_detail_instanceof_event_and_bubbling_phase_constant() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var ev=new CustomEvent('k',{detail:{a:1}}); ev.detail.a === 1 && ev instanceof Event && Event.BUBBLING_PHASE === 3",
    );
}

#[test]
fn redispatching_an_in_flight_event_is_an_invalid_state_error() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var e2=new Event('r'), inner=null; document.body.addEventListener('r', ()=>{ try { document.body.dispatchEvent(e2); } catch(x) { inner = x.name; } }); document.body.dispatchEvent(e2); inner === 'InvalidStateError'",
    );
}

#[test]
fn a_handleevent_object_listener_is_called_with_itself_as_this() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var h={handleEvent(e){ this.hit=e.type; }}; document.body.addEventListener('q', h); document.body.dispatchEvent(new Event('q')); h.hit === 'q'",
    );
}

#[test]
fn timer_callback_exception_reaches_the_window_error_event_and_is_recorded() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var seen=null; window.addEventListener('error', e=>{ seen=e.message; }); \
         setTimeout(()=>{ throw new Error('timer-boom'); }, 0); true",
    );
    rt.run_until_idle().unwrap();
    ok(&mut rt, "/timer-boom/.test(seen)");
    let errors = uncaught(&mut rt);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("timer-boom"), "{errors:?}");
}

// ---- stopPropagation / stopImmediatePropagation ---------------------------

#[test]
fn stop_propagation_during_capture_stops_before_the_next_node_including_the_target() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log=[]; var b=document.body; var p=document.createElement('p'); b.appendChild(p);
  document.addEventListener('cap', e=>{ log.push('doc'); e.stopPropagation(); }, true);
  b.addEventListener('cap', ()=>log.push('body-capture'), true);
  p.addEventListener('cap', ()=>log.push('target'));
  p.dispatchEvent(new Event('cap')); log.join() === 'doc'",
    );
}

#[test]
fn stop_immediate_propagation_stops_the_remaining_listeners_on_the_same_node() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log=[]; document.body.addEventListener('si', e=>{ log.push('a'); e.stopImmediatePropagation(); });
  document.body.addEventListener('si', ()=>log.push('b'));
  document.body.dispatchEvent(new Event('si')); log.join() === 'a'",
    );
}

#[test]
fn stop_propagation_at_the_target_still_prevents_bubbling() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log=[]; var b=document.body; var p=document.createElement('p'); b.appendChild(p);
  p.addEventListener('sp', e=>{ log.push('target'); e.stopPropagation(); });
  b.addEventListener('sp', ()=>log.push('bubble'));
  p.dispatchEvent(new Event('sp', {bubbles:true})); log.join() === 'target'",
    );
}

#[test]
fn a_non_bubbling_event_never_reaches_ancestor_listeners() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log=[]; var b=document.body; var p=document.createElement('p'); b.appendChild(p);
  p.addEventListener('nb', ()=>log.push('target'));
  b.addEventListener('nb', ()=>log.push('bubble'));
  p.dispatchEvent(new Event('nb')); log.join() === 'target'",
    );
}

// ---- constructors ----------------------------------------------------------

#[test]
fn event_customevent_errorevent_all_require_new() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { Event('x'); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { CustomEvent('x'); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "try { ErrorEvent('x'); false } catch (e) { e instanceof TypeError }",
    );
}

#[test]
fn event_type_is_a_required_argument() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { new Event(); false } catch (e) { e instanceof TypeError }",
    );
}

#[test]
fn a_non_object_init_dict_is_a_type_error() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { new Event('x', 5); false } catch (e) { e instanceof TypeError }",
    );
}

#[test]
fn event_init_defaults_and_composed_flag() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var e=new Event('x'); !e.bubbles && !e.cancelable && !e.composed && e.isTrusted === false",
    );
    ok(&mut rt, "new Event('x', {composed:true}).composed === true");
}

#[test]
fn custom_event_without_detail_defaults_to_undefined() {
    let mut rt = rt();
    ok(&mut rt, "new CustomEvent('x').detail === undefined");
}

#[test]
fn error_event_fields_round_trip() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var boxed = {}; var ev = new ErrorEvent('error', {message:'m', filename:'f', lineno:1, colno:2, error:boxed}); \
         ev.message === 'm' && ev.filename === 'f' && ev.lineno === 1 && ev.colno === 2 && ev.error === boxed",
    );
}

#[test]
fn event_phase_constants_on_constructor_and_prototype() {
    let mut rt = rt();
    ok(
        &mut rt,
        "Event.NONE === 0 && Event.CAPTURING_PHASE === 1 && Event.AT_TARGET === 2 && Event.BUBBLING_PHASE === 3 && \
         Event.prototype.NONE === 0 && Event.prototype.BUBBLING_PHASE === 3",
    );
}

// ---- target / currentTarget / eventPhase lifecycle ------------------------

#[test]
fn target_current_target_and_phase_reset_after_dispatch() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var seenPhase=null, seenCurrent=null; \
         document.body.addEventListener('x', e=>{ seenPhase=e.eventPhase; seenCurrent = e.currentTarget === document.body; }); \
         var e=new Event('x'); document.body.dispatchEvent(e); \
         seenPhase === Event.AT_TARGET && seenCurrent && e.eventPhase === Event.NONE && e.currentTarget === null && e.target === document.body",
    );
}

#[test]
fn composed_path_is_empty_outside_dispatch_and_lists_the_path_during_it() {
    let mut rt = rt();
    ok(&mut rt, "new Event('x').composedPath().length === 0");
    ok(
        &mut rt,
        "var seenLen=0, seenFirst=false; var b=document.body; var p=document.createElement('p'); b.appendChild(p); \
         p.addEventListener('x', e=>{ var path=e.composedPath(); seenLen=path.length; seenFirst = path[0] === p; }); \
         p.dispatchEvent(new Event('x', {bubbles:true})); seenLen === 5 && seenFirst",
    );
}

#[test]
fn dispatch_event_returns_false_when_a_cancelable_default_was_prevented() {
    let mut rt = rt();
    ok(
        &mut rt,
        "document.body.addEventListener('x', e=>e.preventDefault()); \
         document.body.dispatchEvent(new Event('x', {cancelable:true})) === false",
    );
}

#[test]
fn prevent_default_on_a_non_cancelable_event_is_a_no_op() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var e=new Event('x'); document.body.addEventListener('x', ()=>e.preventDefault()); \
         document.body.dispatchEvent(e) === true && !e.defaultPrevented",
    );
}

#[test]
fn dispatch_event_with_a_non_event_argument_is_a_type_error() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { document.body.dispatchEvent({}); false } catch (e) { e instanceof TypeError }",
    );
}

#[test]
fn window_dispatch_event_dispatches_at_the_window_alone() {
    let mut rt = rt();
    ok(&mut rt, "window.dispatchEvent(new Event('foo')) === true");
}

// ---- event handler IDL attributes -----------------------------------------

#[test]
fn onclick_setter_registers_at_the_position_first_assigned() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log=[]; document.body.onclick=()=>log.push('a'); document.body.addEventListener('click', ()=>log.push('b')); \
         document.body.onclick=()=>log.push('c'); document.body.dispatchEvent(new Event('click')); log.join() === 'c,b'",
    );
}

#[test]
fn onclick_getter_reflects_the_current_handler_or_null() {
    let mut rt = rt();
    ok(&mut rt, "document.body.onclick === null");
    ok(
        &mut rt,
        "function f(){} document.body.onclick=f; document.body.onclick === f",
    );
}

#[test]
fn setting_a_non_function_handler_clears_it() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var called=false; document.body.onclick=()=>{ called=true; }; document.body.onclick=null; \
         document.body.dispatchEvent(new Event('click')); !called && document.body.onclick === null",
    );
}

#[test]
fn oninput_and_onchange_are_also_available_on_html_elements() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var i=0, c=0; document.body.oninput=()=>i++; document.body.onchange=()=>c++; \
         document.body.dispatchEvent(new Event('input')); document.body.dispatchEvent(new Event('change')); \
         i === 1 && c === 1",
    );
}

#[test]
fn window_gets_the_same_five_handler_attributes() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var hit=false; window.onload=()=>{ hit=true; }; window.dispatchEvent(new Event('load')); hit",
    );
    ok(
        &mut rt,
        "window.onclick === null && window.oninput === null && window.onchange === null && window.onerror === null",
    );
}

// ---- recursive report_exception guard -------------------------------------

#[test]
fn a_throwing_onerror_handler_is_recorded_directly_without_recursing() {
    let mut rt = rt();
    ok(
        &mut rt,
        "window.onerror = function(){ throw new Error('nested'); }; \
         document.body.addEventListener('boom', ()=>{ throw new Error('boom'); }); \
         document.body.dispatchEvent(new Event('boom')); true",
    );
    let errors = uncaught(&mut rt);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].contains("nested"), "{errors:?}");
    assert!(errors[1].contains("boom"), "{errors:?}");
}

// ---- fire_event / report_exception (Rust-side entry points) ---------------

#[test]
fn fire_event_is_trusted_and_dispatches_at_the_given_target() {
    let mut rt = rt();
    let body = body_index(&mut rt);
    ok(
        &mut rt,
        "var seen=null; document.body.addEventListener('ping', e=>{ seen = e.isTrusted; }); true",
    );
    let handled = dispatch::fire_event(rt.context_mut(), Some(body), "ping", false, false).unwrap();
    assert!(handled);
    ok(&mut rt, "seen === true");
}

#[test]
fn report_exception_carries_the_source_hint_into_the_error_event_filename() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var fn=null; window.addEventListener('error', e=>{ fn = e.filename; }); true",
    );
    let error = JsError::from_opaque(JsValue::from(JsString::from("boom")));
    dispatch::report_exception(rt.context_mut(), &error, Some("script.js"));
    ok(&mut rt, "fn === 'script.js'");
}

#[test]
fn events_added_with_add_event_listener_after_this_module_exists_still_work() {
    // A basic sanity check that ordinary `add`/`removeEventListener`
    // (`events.rs`) still cooperate with dispatch after `Listener` grew the
    // `is_handler` field.
    let mut rt = rt();
    ok(
        &mut rt,
        "var c=0; function f(){c++;} document.body.addEventListener('e', f); \
         document.body.dispatchEvent(new Event('e')); document.body.removeEventListener('e', f); \
         document.body.dispatchEvent(new Event('e')); c === 1",
    );
}

// ---- coverage: dictionary defaults, timeStamp, and defensive branches -----

#[test]
fn error_event_defaults_when_the_init_dict_is_omitted_entirely() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var e = new ErrorEvent('x'); e.message === '' && e.filename === '' && \
         e.lineno === 0 && e.colno === 0 && e.error === undefined",
    );
}

#[test]
fn error_event_defaults_when_the_init_dict_is_present_but_empty() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var e = new ErrorEvent('x', {}); e.message === '' && e.filename === '' && \
         e.lineno === 0 && e.colno === 0",
    );
}

#[test]
fn event_time_stamp_is_the_virtual_clock_value_at_construction() {
    let mut rt = rt();
    ok(&mut rt, "new Event('x').timeStamp === 0");
}

#[test]
fn a_listener_without_a_call_or_handleevent_method_is_a_silent_no_op() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var h={}; document.body.addEventListener('t', h); \
         document.body.dispatchEvent(new Event('t')); true",
    );
}

#[test]
fn stop_propagation_during_bubble_stops_before_the_next_ancestor() {
    let mut rt = rt();
    ok(
        &mut rt,
        "var log=[]; var b=document.body; var p=document.createElement('p'); b.appendChild(p);
  p.addEventListener('bp', ()=>log.push('target'));
  b.addEventListener('bp', e=>{ log.push('body'); e.stopPropagation(); });
  document.addEventListener('bp', ()=>log.push('doc'));
  p.dispatchEvent(new Event('bp', {bubbles:true})); log.join() === 'target,body'",
    );
}

#[test]
fn setting_a_handler_to_null_with_no_existing_handler_is_a_no_op() {
    let mut rt = rt();
    ok(
        &mut rt,
        "document.body.onclick = null; document.body.onclick === null",
    );
}

#[test]
fn a_resource_limit_hit_inside_a_listener_aborts_dispatch_uncatchably() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::with_options(
        host,
        RunOptions {
            limits: Limits {
                max_loop_iterations: 10,
                ..Default::default()
            },
        },
    )
    .unwrap();
    let result = rt.evaluate(
        "try { \
           document.body.addEventListener('x', ()=>{ while (true) {} }); \
           document.body.dispatchEvent(new Event('x')); \
           'not aborted' \
         } catch (e) { 'caught' }",
    );
    assert_eq!(result, Err(RuntimeError::Aborted(Abort::LoopIterations)));
    assert_eq!(rt.run_until_idle(), Err(Abort::LoopIterations));
}

/// Both of these are unreachable from script (the exposed `dispatchEvent`
/// binding brand-checks its argument before ever calling
/// [`dispatch::dispatch`], and every internal caller always passes a
/// freshly built `Event`/`CustomEvent`/`ErrorEvent`), but the defensive
/// checks themselves are still reachable directly, and worth pinning down
/// rather than left as dead code no test ever proves correct.
#[test]
fn dispatch_and_call_window_on_error_on_a_non_event_object_are_defensive_no_ops() {
    let mut rt = rt();
    let body = rt.evaluate("document.body").unwrap().as_object().unwrap();
    let callback = rt
        .evaluate("(function(){ throw new Error('should not run'); })")
        .unwrap()
        .as_object()
        .unwrap();
    let context = rt.context_mut();
    let err = dispatch::dispatch(context, &body, None);
    assert!(err.is_err());
    let result = dispatch::call_window_on_error(context, &callback, &body).unwrap();
    assert!(result.is_undefined());
}
