use super::*;
use crate::runtime::DomRuntime;
use crate::runtime::interfaces::wrap;
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;
use boa_engine::JsValue;
use boa_engine::object::JsObject;
use raikiri_style::{StyleDom, StyleElement, StyleNode, StyleNodeId};

fn runtime_with_target() -> (DomRuntime, usize) {
    let (mut host, _, _, body) = StubHost::page();
    let target = host.document.create_detached_element("div").unwrap();
    host.document
        .set_element_attribute(target, "id", "target")
        .unwrap();
    host.document
        .set_element_inline_style(target, Some("font-size: 12px".into()));
    host.document.append_child(body, target).unwrap();
    (DomRuntime::new(host).unwrap(), target)
}

fn animation_style(runtime: &mut DomRuntime, target: usize) -> Option<String> {
    with_state(runtime.context_mut(), |state| {
        let node = state
            .host
            .document()
            .node(StyleNodeId::new(target as u64))?;
        let element = node.as_element()?;
        Some(element.animation_style_source().map(str::to_owned))
    })
    .unwrap()
    .flatten()
}

fn expose_target(runtime: &mut DomRuntime, target: usize) {
    let object = wrap(runtime.context_mut(), target).unwrap();
    runtime
        .context_mut()
        .register_global_property(
            boa_engine::JsString::from("target"),
            object,
            boa_engine::property::Attribute::all(),
        )
        .unwrap();
}

#[test]
fn samples_linear_midpoint_and_endpoints() {
    let font_size = KeyframePair::new(AnimationProperty::FontSize, 0.0, 40.0, 40.0)
        .expect("valid font-size keyframes");
    assert_eq!(font_size.sample(0.0), Some(0.0));
    assert_eq!(font_size.sample(20.0), Some(20.0));
    assert_eq!(font_size.sample(-0.01), None);
    assert_eq!(font_size.sample(40.0), None);
    assert_eq!(font_size.sample(f64::NAN), None);
    assert_eq!(font_size.sample(f64::INFINITY), None);

    let word_spacing = KeyframePair::new(AnimationProperty::WordSpacing, 0.0, 40.0, 40.0)
        .expect("valid word-spacing keyframes");
    assert_eq!(word_spacing.sample(20.0), Some(20.0));
}

#[test]
fn rejects_unsupported_property_values_and_invalid_durations() {
    assert!(AnimationProperty::parse_cssom_name("opacity").is_err());

    for duration in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(
            KeyframePair::new(AnimationProperty::FontSize, 0.0, 40.0, duration).is_err(),
            "duration {duration:?} must be rejected"
        );
    }

    for value in [f64::NAN, f64::INFINITY] {
        assert!(
            KeyframePair::new(AnimationProperty::FontSize, value, 40.0, 40.0).is_err(),
            "keyframe {value:?} must be rejected"
        );
    }
}

#[test]
fn zero_duration_has_no_active_sample() {
    let effect = KeyframePair::new(AnimationProperty::WordSpacing, 0.0, 40.0, 0.0)
        .expect("zero duration is valid");
    assert_eq!(effect.sample(0.0), None);
}

#[test]
fn finite_extreme_keyframes_do_not_overflow_during_interpolation() {
    let effect = KeyframePair::new(AnimationProperty::FontSize, -f64::MAX, f64::MAX, 2.0)
        .expect("finite endpoints are valid");
    assert_eq!(effect.sample(1.0), Some(0.0));
}

#[test]
fn exact_word_spacing_animation_fixtures_pause_and_seek_to_midpoint() {
    let fixtures = [
        (
            "const target = document.querySelector('#target');\n\
             const animation = target.animate({ fontSize: ['0px', '40px'] }, 40);\n\
             animation.pause();\n\
             animation.currentTime = 20;\n\
             animation instanceof Animation && animation.currentTime === 20",
            "font-size: 20px;",
        ),
        (
            "const target = document.querySelector('#target');\n\
             const animation = target.animate({ wordSpacing: ['0px', '40px'] }, 40);\n\
             animation.pause();\n\
             animation.currentTime = 20;\n\
             animation instanceof Animation && animation.currentTime === 20",
            "word-spacing: 20px;",
        ),
    ];

    for (script, expected_style) in fixtures {
        let (mut runtime, target) = runtime_with_target();
        let result = runtime
            .evaluate(script)
            .expect("fixture animation script runs");
        assert!(result.to_boolean(), "animation controls must work");
        assert_eq!(
            animation_style(&mut runtime, target).as_deref(),
            Some(expected_style)
        );
    }
}

#[test]
fn animation_sample_does_not_change_authored_style() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "const authoredStyle = target.getAttribute('style');\
             const animation = target.animate({ fontSize: ['0px', '40px'] }, 40);\
             animation.pause();\
             animation.currentTime = 20;\
             target.getAttribute('style') === authoredStyle && target.style.fontSize === '12px'",
        )
        .expect("animation sample preserves authored style");

    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("font-size: 20px;")
    );
    assert!(
        runtime
            .evaluate("target.getAttribute('style') === 'font-size: 12px'")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn seeking_invalidates_style_without_changing_dom_generation() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate("document.body.getBoundingClientRect()")
        .unwrap();
    let before = with_state(runtime.context_mut(), |state| {
        (state.dirty, state.generation)
    })
    .unwrap();
    assert!(!before.0);

    runtime
        .evaluate(
            "globalThis.animation = target.animate({ wordSpacing: ['0px', '40px'] }, 40);\
             animation.pause();\
             animation.currentTime = 20",
        )
        .expect("animation sample is accepted");

    let after = with_state(runtime.context_mut(), |state| {
        (state.dirty, state.generation)
    })
    .unwrap();
    assert!(after.0, "animation sampling must schedule a host flush");
    assert_eq!(
        after.1, before.1,
        "animation sampling is not a DOM mutation"
    );
}

#[test]
fn seeking_outside_the_active_interval_clears_the_previous_sample() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.animation = target.animate({ wordSpacing: ['0px', '40px'] }, 40);\
             animation.pause();\
             animation.currentTime = 20",
        )
        .unwrap();
    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("word-spacing: 20px;")
    );

    runtime.evaluate("animation.currentTime = 40").unwrap();
    assert_eq!(animation_style(&mut runtime, target), None);

    runtime.evaluate("animation.currentTime = -1").unwrap();
    assert_eq!(animation_style(&mut runtime, target), None);
}

#[test]
fn later_created_replace_effect_wins_for_the_same_property() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.firstAnimation = target.animate({ wordSpacing: ['0px', '40px'] }, 40);\
             globalThis.secondAnimation = target.animate({ wordSpacing: ['10px', '50px'] }, 40);\
             firstAnimation.pause();\
             firstAnimation.currentTime = 20;\
             secondAnimation.pause();\
             secondAnimation.currentTime = 20",
        )
        .unwrap();

    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("word-spacing: 20px; word-spacing: 30px;")
    );
}

#[test]
fn unsupported_animation_inputs_throw_type_error() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const validAnimation = target.animate({ fontSize: ['0px', '40px'] }, 40);\
             validAnimation.pause();\
             [\
               () => target.animate({ fontSize: ['0px', '40px'] }, { duration: -1 }),\
               () => target.animate({ fontSize: ['0px', '40px'] }, { duration: 'slow' }),\
               () => target.animate({ fontSize: ['0px', '40px'] }, 'slow'),\
               () => target.animate({ opacity: [0, 1] }, -1),\
               () => target.animate(1, 40),\
               () => target.animate(),\
               () => target.animate({ fontSize: ['0px', '40px'] }, -1),\
               () => target.animate({ fontSize: ['0px', '40px'] }, NaN),\
               () => target.animate({ fontSize: ['0px', '40px'] }, Infinity),\
               () => target.animate({ fontSize: ['0px', '40px'] }, 40).pause.call({})\
             ].every(run => { try { run(); return false; } catch (error) { return error instanceof TypeError; } })",
        )
        .expect("invalid animation inputs are caught as TypeErrors");

    assert!(
        result.to_boolean(),
        "every unsupported input must throw TypeError"
    );
}

#[test]
fn css_number_syntax_accepts_only_complete_css_numbers() {
    for value in ["0", "+1", "-2", "1.5", ".5", "1e2", "1.5E-2"] {
        assert!(is_css_number_syntax(value), "{value:?} should be accepted");
    }
    for value in ["", "+", ". ", "1.", "1e", "1e+", "1  ", "x"] {
        assert!(!is_css_number_syntax(value), "{value:?} should be rejected");
    }
}

#[test]
fn stale_animation_ids_are_rejected_by_controls() {
    let (mut runtime, _) = runtime_with_target();
    let prototype = super::super::interfaces::protos(runtime.context_mut())
        .animation
        .clone();
    let data = AnimationData {
        id: usize::MAX,
        animation_id: String::new(),
        ready: None,
        finished: None,
        finish_resolve: None,
    };
    let animation = JsObject::from_proto_and_data(Some(prototype), data);
    let animation: JsValue = animation.into();

    assert!(pause(&animation, &[], runtime.context_mut()).is_err());
    assert!(set_current_time(&animation, &[JsValue::from(1.0)], runtime.context_mut()).is_err());
    assert!(finished(&animation, &[], runtime.context_mut()).is_err());
}

#[test]
fn paused_effect_without_a_current_time_has_no_sample() {
    let (mut runtime, target) = runtime_with_target();
    with_state(runtime.context_mut(), |state| {
        state.animations.push(AnimationEffect {
            target,
            keyframes: Some(
                KeyframePair::new(AnimationProperty::FontSize, 0.0, 40.0, 40.0)
                    .expect("valid keyframes"),
            ),
            start_time_ms: 0.0,
            end_time_ms: 40.0,
            current_time_ms: None,
            paused: true,
            detached: false,
            finish_task: None,
        });
    })
    .unwrap();

    refresh_animation_styles(runtime.context_mut()).unwrap();
    assert_eq!(animation_style(&mut runtime, target), None);
}

#[test]
fn finite_numeric_duration_accepts_the_javascript_double_range() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const animation = target.animate({ fontSize: ['0px', '40px'] }, Number.MAX_VALUE);\
             animation.pause();\
             animation.currentTime = 1;\
             animation instanceof Animation && animation.currentTime === 1",
        )
        .expect("finite numeric duration is accepted");

    assert!(result.to_boolean());
}

#[test]
fn options_dictionary_sets_the_sampled_duration() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "const animation = target.animate({ wordSpacing: ['0px', '40px'] }, { duration: 40 });\
             animation.pause();\
             animation.currentTime = 10",
        )
        .expect("a dictionary duration is accepted");
    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("word-spacing: 10px;")
    );
}

#[test]
fn absent_and_auto_durations_are_zero() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "const animations = [\
               target.animate({ wordSpacing: ['0px', '40px'] }),\
               target.animate({ wordSpacing: ['0px', '40px'] }, undefined),\
               target.animate({ wordSpacing: ['0px', '40px'] }, {}),\
               target.animate({ wordSpacing: ['0px', '40px'] }, { duration: 'auto' }),\
               target.animate({ wordSpacing: ['0px', '40px'] }, null),\
               target.animate({ wordSpacing: ['0px', '40px'] }, false)\
             ];\
             for (const animation of animations) { animation.pause(); animation.currentTime = 0; }",
        )
        .expect("absent and auto durations are accepted");
    assert_eq!(animation_style(&mut runtime, target), None);
}

#[test]
fn keyframes_outside_the_sampler_keep_timing_but_never_style() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const animations = [\
               target.animate({ opacity: [0, 1] }, { duration: 1 }),\
               target.animate({}, 40),\
               target.animate(null, 40),\
               target.animate([{ opacity: 0 }, { opacity: 1 }], 40),\
               target.animate({ fontSize: ['0px', '40px'], wordSpacing: ['0px', '40px'] }, 40),\
               target.animate({ fontSize: '10px' }, 40),\
               target.animate({ fontSize: ['0px'] }, 40),\
               target.animate({ fontSize: ['0px', '20px', '40px'] }, 40),\
               target.animate({ fontSize: [0, '40px'] }, 40),\
               target.animate({ fontSize: ['0 px', '40px'] }, 40),\
               target.animate({ fontSize: ['0px', '1.px'] }, 40),\
               target.animate({ fontSize: ['', '40px'] }, 40),\
               target.animate({ fontSize: ['0', '40px'] }, 40),\
               target.animate({ wordSpacing: ['0em', '1em'] }, 40)\
             ];\
             animations.every(animation => {\
               const refused = run => { try { run(); return false; } catch (error) { return error instanceof TypeError; } };\
               return animation instanceof Animation\
                 && animation.currentTime === 0\
                 && refused(() => animation.pause())\
                 && refused(() => { animation.currentTime = 0; });\
             })",
        )
        .expect("keyframes outside the sampler are accepted");
    assert!(result.to_boolean(), "only sampling is refused");
    assert_eq!(animation_style(&mut runtime, target), None);
}

#[test]
fn ready_is_one_resolved_promise_for_the_animation() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.animation = target.animate({ opacity: [0, 1] }, 40);\
             globalThis.sameReady = animation.ready === animation.ready;\
             animation.ready.then(value => { globalThis.readyValue = value; })",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate("sameReady && readyValue === animation")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn finished_resolves_at_the_end_of_the_active_interval() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.animation = target.animate({ opacity: [0, 1] }, { duration: 40 });\
             globalThis.sameFinished = animation.finished === animation.finished;\
             animation.finished.then(value => {\
               globalThis.finishedValue = value;\
               globalThis.finishedAt = performance.now();\
             })",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate("sameFinished && finishedValue === animation && finishedAt === 40")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn finished_counts_from_the_start_time_not_the_first_access() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.animation = target.animate({ opacity: [0, 1] }, { duration: 40 });\
             setTimeout(() => {\
               animation.finished.then(() => { globalThis.finishedAt = performance.now(); });\
             }, 10);\
             setTimeout(() => {\
               const late = target.animate({ opacity: [0, 1] }, 5);\
               late.finished.then(() => { globalThis.lateAt = performance.now(); });\
             }, 60)",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate("finishedAt === 40 && lateAt === 65")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn paused_animation_never_finishes() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.animation = target.animate({ wordSpacing: ['0px', '40px'] }, 40);\
             animation.finished.then(() => { globalThis.finishedValue = true; });\
             animation.pause()",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate("typeof finishedValue === 'undefined'")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn promise_getters_reject_foreign_receivers() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const animation = target.animate({ opacity: [0, 1] }, 40);\
             const proto = Object.getPrototypeOf(animation);\
             ['ready', 'finished'].every(name => {\
               const getter = Object.getOwnPropertyDescriptor(proto, name).get;\
               try { getter.call({}); return false; } catch (error) { return error instanceof TypeError; }\
             })",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn primitive_options_convert_to_a_number_but_durations_do_not() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "const fromString = target.animate({ wordSpacing: ['0px', '40px'] }, '40');\
             fromString.pause();\
             fromString.currentTime = 10;\
             const autoObject = { toString: () => 'auto' };\
             const fromAuto = target.animate({ opacity: [0, 1] }, { duration: autoObject });\
             fromAuto.finished.then(() => { globalThis.autoAt = performance.now(); });\
             globalThis.refusedDurations = [true, null, 'AUTO', '40'].every(duration => {\
               try { target.animate({ opacity: [0, 1] }, { duration }); return false; }\
               catch (error) { return error instanceof TypeError; }\
             })",
        )
        .expect("primitive options convert to numbers");
    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("word-spacing: 10px;")
    );
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate("autoAt === 0 && refusedDurations")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn pausing_cancels_a_long_finishing_task() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.animation = target.animate({ wordSpacing: ['0px', '40px'] }, 100000);\
             animation.finished.then(() => { globalThis.finishedValue = true; });\
             animation.pause()",
        )
        .unwrap();
    runtime
        .run_until_idle()
        .expect("a cancelled finishing task does not hit the virtual-time limit");
    assert!(
        runtime
            .evaluate("typeof finishedValue === 'undefined' && performance.now() === 0")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn reading_finished_after_the_end_resolves_at_once() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.order = [];\
             const animation = target.animate({ opacity: [0, 1] }, 10);\
             setTimeout(() => {\
               animation.finished.then(() => order.push('finished'));\
               queueMicrotask(() => order.push('later'));\
             }, 20)",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate("order.join() === 'finished,later'")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn reading_finished_while_paused_queues_nothing() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.animation = target.animate({ wordSpacing: ['0px', '40px'] }, 100000);\
             animation.pause();\
             animation.finished.then(() => { globalThis.finishedValue = true; })",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate("typeof finishedValue === 'undefined'")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn timing_members_set_the_finish_time() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.times = {};\
             const watch = (name, options) => target.animate({ opacity: [0, 1] }, options)\
               .finished.then(() => { times[name] = performance.now(); });\
             watch('iterations', { duration: 10, iterations: 3 });\
             watch('delays', { duration: 10, delay: 5, endDelay: 7 });\
             watch('negative', { duration: 10, delay: -20 });\
             watch('zeroForever', { duration: 0, iterations: Infinity });\
             watch('forever', { duration: 10, iterations: Infinity })",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate(
                "times.iterations === 30 && times.delays === 22 && times.negative === 0\
                 && times.zeroForever === 0 && !('forever' in times)"
            )
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn invalid_timing_and_unsampled_timing_throw() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "[\
               () => target.animate({ opacity: [0, 1] }, { iterations: -1 }),\
               () => target.animate({ opacity: [0, 1] }, { iterations: NaN }),\
               () => target.animate({ opacity: [0, 1] }, { delay: Infinity }),\
               () => target.animate({ opacity: [0, 1] }, { endDelay: NaN }),\
               () => target.animate({ wordSpacing: ['0px', '40px'] }, { duration: 40, delay: 1 }),\
               () => target.animate({ wordSpacing: ['0px', '40px'] }, { duration: 40, endDelay: 1 }),\
               () => target.animate({ wordSpacing: ['0px', '40px'] }, { duration: 40, iterations: 2 })\
             ].every(run => { try { run(); return false; } catch (error) { return error instanceof TypeError; } })",
        )
        .unwrap();
    assert!(result.to_boolean());
    runtime
        .evaluate(
            "target.animate({ wordSpacing: ['0px', '40px'] }, { duration: 40, delay: 0, endDelay: 0, iterations: 1 })",
        )
        .expect("default timing members keep a sampled effect");
}

#[test]
fn keyframe_sequences_are_validated() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const refused = keyframes => {\
               try { target.animate(keyframes, 1); return false; }\
               catch (error) { return error instanceof TypeError; }\
             };\
             [[1], [{ offset: 2 }], [{ offset: -1 }], [{ offset: 0.6 }, { offset: 0.4 }], [{ offset: NaN }]]\
               .every(refused)\
             && !refused([null, undefined, { offset: null }, { offset: 0 }, {}, { offset: '0.5' }, { offset: 1 }])",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn effect_timing_members_are_validated_in_dictionary_order() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const refused = options => {\
               try { target.animate(null, options); return false; }\
               catch (error) { return error instanceof TypeError; }\
             };\
             const read = [];\
             target.animate(null, new Proxy({}, { get(_, name) { read.push(name); } }));\
             read.join() === 'delay,direction,duration,easing,endDelay,fill,iterationStart,iterations,composite,iterationComposite,pseudoElement,id,timeline'\
             && [{ easing: 'bogus' }, { easing: '' }, { direction: 'sideways' }, { fill: 'x' },\
                 { iterationStart: -1 }, { iterationStart: NaN }, { composite: 'auto' },\
                 { iterationComposite: 'add' }].every(refused)\
             && !refused({ direction: 'alternate-reverse', easing: 'steps(2, jump-none)', fill: 'both',\
                           iterationStart: 0.5, composite: 'add', iterationComposite: 'accumulate' })",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn sampled_effects_require_default_timing_shape() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const refused = options => {\
               try { target.animate({ wordSpacing: ['0px', '40px'] }, options); return false; }\
               catch (error) { return error instanceof TypeError; }\
             };\
             [{ duration: 40, easing: 'ease' }, { duration: 40, direction: 'reverse' },\
              { duration: 40, fill: 'forwards' }, { duration: 40, composite: 'add' },\
              { duration: 40, iterationStart: 0.5 }, { duration: Infinity }].every(refused)\
             && !refused({ duration: 40, easing: ' LINEAR ', direction: 'normal', fill: 'auto', composite: 'replace' })\
             && !refused({ duration: 40, fill: 'none', iterationComposite: 'accumulate' })",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn unsampled_effects_accept_an_infinite_duration() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.times = {};\
             target.animate(null, Infinity).finished.then(() => { times.forever = true; });\
             target.animate(null, { duration: Infinity, iterations: 0 })\
               .finished.then(() => { times.none = performance.now(); });\
             globalThis.refused = [-Infinity, NaN].every(duration => {\
               try { target.animate(null, { duration }); return false; }\
               catch (error) { return error instanceof TypeError; }\
             })",
        )
        .expect("an infinite duration is valid for unsampled keyframes");
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate("refused && times.none === 0 && !('forever' in times)")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn keyframe_sequence_easing_and_composite_are_validated() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const refused = keyframes => {\
               try { target.animate(keyframes, 1); return false; }\
               catch (error) { return error instanceof TypeError; }\
             };\
             [[{ easing: 'bogus' }], [{ composite: 'x' }], [{ easing: 'linear' }, { easing: 'steps(0)' }]]\
               .every(refused)\
             && !refused([{ easing: 'steps(2)', composite: 'auto' }, { easing: 'cubic-bezier(0, 2, 1, -1)' }])",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn seeking_a_running_animation_reschedules_finished() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.times = {};\
             const watch = (name, seek) => {\
               const animation = target.animate({ wordSpacing: ['0px', '40px'] }, 100);\
               animation.finished.then(() => { times[name] = performance.now(); });\
               setTimeout(() => { animation.currentTime = seek; }, 10);\
               return animation;\
             };\
             watch('forward', 90);\
             watch('backward', 0);\
             watch('past', 200);\
             const done = target.animate({ wordSpacing: ['0px', '40px'] }, 5);\
             done.finished.then(() => { done.currentTime = 0; times.done = performance.now(); })",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate(
                "times.forward === 20 && times.backward === 110 && times.past === 10\
                 && times.done === 5"
            )
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn keyframe_iterables_are_consumed_through_their_iterator() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const refused = keyframes => {\
               try { target.animate(keyframes, 1); return false; }\
               catch (error) { return error instanceof TypeError; }\
             };\
             const iterable = iterator => ({ [Symbol.iterator]: () => iterator });\
             const thrown = {};\
             let rethrown = false;\
             try { target.animate({ *[Symbol.iterator]() { throw thrown; } }, 1); }\
             catch (error) { rethrown = error === thrown; }\
             rethrown\
             && [new Set([{ offset: 2 }]), { [Symbol.iterator]: 1 }, iterable(1),\
                 iterable({ next: 1 }), iterable({ next: () => 1 })].every(refused)\
             && !refused(new Set([{ offset: 0 }, { offset: 1 }]))\
             && !refused({ *[Symbol.iterator]() { yield { easing: 'ease' }; yield null; } })",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn leaving_the_finished_state_replaces_the_finished_promise() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.times = {};\
             const replay = target.animate({ wordSpacing: ['0px', '40px'] }, 10);\
             const first = replay.finished;\
             first.then(() => {\
               times.first = performance.now();\
               replay.currentTime = 20;\
               times.samePastEnd = replay.finished === first;\
               replay.currentTime = 0;\
               times.replaced = replay.finished !== first;\
               replay.finished.then(() => { times.second = performance.now(); });\
             });\
             const paused = target.animate({ wordSpacing: ['0px', '40px'] }, 5);\
             const before = paused.finished;\
             before.then(() => {\
               paused.pause();\
               times.pausedReplaced = paused.finished !== before;\
               paused.finished.then(() => { times.pausedFinished = true; });\
             })",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate(
                "times.first === 10 && times.samePastEnd && times.replaced && times.second === 20\
                 && times.pausedReplaced && !('pausedFinished' in times)"
            )
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn property_indexed_keyframe_members_are_validated() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const refused = keyframes => {\
               try { target.animate(keyframes, 1); return false; }\
               catch (error) { return error instanceof TypeError; }\
             };\
             [{ easing: 'bogus' }, { easing: ['linear', 'bogus'] }, { composite: 'x' },\
              { composite: new Set(['add', 'x']) }, { offset: 2 }, { offset: [0.6, 0.4] },\
              { offset: NaN }, { opacity: [0, 1], easing: { [Symbol.iterator]: 1 } }].every(refused)\
             && !refused({ opacity: [0, 1], easing: ['ease', 'steps(2)'], composite: 'add',\
                           offset: [0, null, 1] })\
             && !refused({ color: 'red', easing: 'linear', composite: ['auto'], offset: null })",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn animation_options_convert_pseudo_element_id_and_timeline() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "const thrown = {};\
             const error = options => {\
               try { target.animate(null, options); return null; } catch (error) { return error; }\
             };\
             globalThis.results = {\
               id: error({ get id() { throw thrown; } }) === thrown,\
               timeline: error({ timeline: {} }) instanceof TypeError,\
               pseudo: ['', 'before', ':marker', '::abc', '::part(x)']\
                 .every(pseudoElement => error({ pseudoElement })?.name === 'SyntaxError'),\
               validPseudo: ['::before', '::AFTER', '::marker', ':first-line', null]\
                 .every(pseudoElement => error({ pseudoElement, id: 7 }) === null),\
               sampledPseudo: (() => {\
                 try { target.animate({ wordSpacing: ['0px', '1px'] }, { pseudoElement: '::before' }); return false; }\
                 catch (error) { return error instanceof TypeError; }\
               })(),\
             };\
             const detached = target.animate(null, { duration: 10, timeline: null });\
             results.detachedTime = detached.currentTime === null;\
             detached.ready.then(() => { results.readySettled = true; });\
             detached.finished.then(() => { results.finishedSettled = true; })",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate(
                "results.id && results.timeline && results.pseudo && results.validPseudo\
                 && results.sampledPseudo && results.detachedTime\
                 && !('readySettled' in results) && !('finishedSettled' in results)"
            )
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn options_are_validated_before_keyframes() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const read = [];\
             const keyframes = { get offset() { read.push('offset'); } };\
             try { target.animate(keyframes, { get delay() { read.push('delay'); }, iterations: -1 }); }\
             catch (error) { read.push(error instanceof TypeError); }\
             read.join() === 'delay,true'",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn default_property_indexed_metadata_keeps_the_sampler() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "const sampled = target.animate({ wordSpacing: ['0px', '40px'], offset: [0, 1, 1],\
                                               easing: ' Linear ', composite: ['auto', 'replace'] }, 40);\
             sampled.pause();\
             sampled.currentTime = 10;\
             globalThis.unsampled = [{ offset: [0, 0.5] }, { offset: [0.5, 1] }, { easing: 'ease' },\
                                     { composite: 'add' }, { opacity: ['0', '1'] }].every(metadata => {\
               const animation = target.animate({ fontSize: ['0px', '40px'], ...metadata }, 40);\
               try { animation.pause(); return false; } catch (error) { return error instanceof TypeError; }\
             })",
        )
        .unwrap();
    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("word-spacing: 10px;")
    );
    assert!(runtime.evaluate("unsampled").unwrap().to_boolean());
}

#[test]
fn pausing_holds_the_elapsed_time() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.animation = target.animate({ wordSpacing: ['0px', '40px'] }, 100);\
             globalThis.times = { start: animation.currentTime };\
             setTimeout(() => {\
               times.running = animation.currentTime;\
               animation.pause();\
               times.paused = animation.currentTime;\
               animation.pause();\
             }, 50);\
             setTimeout(() => { times.later = animation.currentTime; }, 70)",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate(
                "times.start === 0 && times.running === 50 && times.paused === 50 && times.later === 50"
            )
            .unwrap()
            .to_boolean()
    );
    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("word-spacing: 20px;")
    );
}

#[test]
fn keyframe_properties_are_converted_in_idl_name_order() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const marker = {};\
             const thrown = run => { try { run(); return null; } catch (error) { return error; } };\
             const read = [];\
             const logged = name => ({ toString() { read.push(name); return '1px'; } });\
             thrown(() => target.animate([{ opacity: logged('opacity'), color: logged('color'),\
                                             'font-size': logged('dashed'), foo: logged('foo'),\
                                             cssFloat: logged('cssFloat'), willChange: logged('willChange'),\
                                             unicodeBidi: logged('unicodeBidi') }], 1));\
             const order = read.join() === 'color,cssFloat,opacity';\
             const sampled = target.animate({ wordSpacing: ['0px', '40px'], foo: 1, [Symbol('x')]: 1,\
                                              'font-size': 1, float: 1 }, 40);\
             sampled.pause();\
             order\
             && thrown(() => target.animate([{ get opacity() { throw marker; } }], 1)) === marker\
             && thrown(() => target.animate({ get opacity() { throw marker; } }, 1)) === marker\
             && thrown(() => target.animate({ opacity: [0, { toString() { throw marker; } }] }, 1)) === marker\
             && thrown(() => target.animate({ opacity: { [Symbol.iterator]: 1 } }, 1)) instanceof TypeError\
             && thrown(() => target.animate({ wordSpacing: '0px' }, 1)) === null\
             && thrown(() => target.animate({ get willChange() { throw marker; } }, 1)) === null\
             && thrown(() => target.animate({ get writingMode() { throw marker; } }, 1)) === null\
             && thrown(() => target.animate([{ get direction() { throw marker; } }], 1)) === null\
             && thrown(() => target.animate({ get ['--x']() { throw marker; } }, 1)) === marker\
             && thrown(() => target.animate({ get ['--']() { throw marker; } }, 1)) === null\
             && (() => {\
               const original = Object.keys;\
               Object.keys = () => [];\
               const hidden = Object.defineProperty({}, 'opacity', { get() { throw marker; } });\
               const result = thrown(() => target.animate({ get opacity() { throw marker; } }, 1)) === marker\
                 && thrown(() => target.animate(hidden, 1)) === null\
                 && thrown(() => target.animate(new Proxy({ opacity: 1 },\
                      { getOwnPropertyDescriptor() { throw marker; } }), 1)) === marker;\
               Object.keys = original;\
               return result;\
             })()",
        )
        .unwrap();
    assert!(result.to_boolean());
}

#[test]
fn naturally_finished_animations_hold_their_end_time() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime
        .evaluate(
            "globalThis.times = {};\
             const natural = target.animate({ wordSpacing: ['0px', '40px'] }, 10);\
             const sought = target.animate({ wordSpacing: ['0px', '40px'] }, 10);\
             sought.currentTime = 30;\
             setTimeout(() => {\
               times.natural = natural.currentTime;\
               times.sought = sought.currentTime;\
               natural.pause();\
               times.paused = natural.currentTime;\
             }, 20)",
        )
        .unwrap();
    runtime.run_until_idle().unwrap();
    assert!(
        runtime
            .evaluate("times.natural === 10 && times.sought === 30 && times.paused === 10")
            .unwrap()
            .to_boolean()
    );
}

#[test]
fn animation_id_comes_from_options_and_is_writable() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    let result = runtime
        .evaluate(
            "const named = target.animate(null, { id: 'spin' });\
             const unnamed = target.animate(null, 1);\
             const before = named.id === 'spin' && unnamed.id === '';\
             named.id = 42;\
             const getter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(named), 'id').get;\
             let foreign = false;\
             try { getter.call({}); } catch (error) { foreign = error instanceof TypeError; }\
             before && named.id === '42' && foreign",
        )
        .unwrap();
    assert!(result.to_boolean());
}
