use std::collections::{HashMap, HashSet};

use boa_engine::job::NativeJob;
use boa_engine::object::JsObject;
use boa_engine::object::builtins::{JsFunction, JsPromise};
use boa_engine::property::PropertyKey;
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, Trace, js_string,
};

use raikiri_style::is_valid_easing_function;

use super::interfaces::{Members, protos};
use super::webidl::{this_element, with_state};

/// A CSS property supported by the static animation sampler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnimationProperty {
    /// CSS `font-size`.
    FontSize,
    /// CSS `word-spacing`.
    WordSpacing,
}

impl AnimationProperty {
    /// Parse the camel-case property spelling accepted by `Element.animate`.
    pub(crate) fn parse_cssom_name(name: &str) -> Result<Self, AnimationModelError> {
        match name {
            "fontSize" => Ok(Self::FontSize),
            "wordSpacing" => Ok(Self::WordSpacing),
            _ => Err(AnimationModelError),
        }
    }

    fn css_name(self) -> &'static str {
        match self {
            Self::FontSize => "font-size",
            Self::WordSpacing => "word-spacing",
        }
    }
}

/// Invalid input to the bounded animation model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AnimationModelError;

/// Two absolute pixel keyframes over a numeric duration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct KeyframePair {
    pub(crate) property: AnimationProperty,
    pub(crate) from_px: f64,
    pub(crate) to_px: f64,
    pub(crate) duration_ms: f64,
}

/// One runtime-owned effect. Its vector index in [`super::State::animations`]
/// is the stable ID exposed through the native `Animation` object.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AnimationEffect {
    pub(crate) target: usize,
    /// The sampled keyframes, or `None` for keyframes outside the static
    /// sampler: such an effect keeps its timing but never styles its target.
    pub(crate) keyframes: Option<KeyframePair>,
    /// Virtual time at which the effect was created and started playing.
    pub(crate) start_time_ms: f64,
    /// Effect end time relative to the start time; infinite for an effect
    /// that repeats forever.
    pub(crate) end_time_ms: f64,
    pub(crate) current_time_ms: Option<f64>,
    pub(crate) paused: bool,
    /// The queued task that resolves `finished`, cancelled on pause.
    pub(crate) finish_task: Option<u64>,
}

/// Native data behind an `Animation` instance.
#[derive(Debug, Trace, Finalize, JsData)]
pub(crate) struct AnimationData {
    #[unsafe_ignore_trace]
    id: usize,
    /// The `ready` promise, created on first access.
    ready: Option<JsObject>,
    /// The `finished` promise, created on first access.
    finished: Option<JsObject>,
    /// Resolves `finished`; taken when it runs.
    finish_resolve: Option<JsFunction>,
}

impl KeyframePair {
    /// Build a finite, non-negative-duration linear effect.
    pub(crate) fn new(
        property: AnimationProperty,
        from_px: f64,
        to_px: f64,
        duration_ms: f64,
    ) -> Result<Self, AnimationModelError> {
        if !from_px.is_finite()
            || !to_px.is_finite()
            || !duration_ms.is_finite()
            || duration_ms < 0.0
        {
            return Err(AnimationModelError);
        }
        Ok(Self {
            property,
            from_px,
            to_px,
            duration_ms,
        })
    }

    /// Sample the linear effect during its active interval; default fill is none.
    pub(crate) fn sample(self, current_time_ms: f64) -> Option<f64> {
        if !current_time_ms.is_finite()
            || self.duration_ms <= 0.0
            || current_time_ms < 0.0
            || current_time_ms >= self.duration_ms
        {
            return None;
        }
        let progress = current_time_ms / self.duration_ms;
        let delta = self.to_px - self.from_px;
        let value = if delta.is_finite() {
            self.from_px + delta * progress
        } else {
            self.from_px * (1.0 - progress) + self.to_px * progress
        };
        value.is_finite().then_some(value)
    }
}

fn type_error(message: &str) -> boa_engine::JsError {
    JsNativeError::typ().with_message(message.to_owned()).into()
}

fn parse_pixel_value(value: &JsValue) -> JsResult<f64> {
    let Some(value) = value.as_string() else {
        return Err(type_error("animation keyframes must be pixel strings"));
    };
    let value = value.to_std_string_escaped();
    let value = value.trim();
    let Some(unit_start) = value.len().checked_sub(2) else {
        return Err(type_error("animation keyframes must use px units"));
    };
    if !value.as_bytes()[unit_start..].eq_ignore_ascii_case(b"px") {
        return Err(type_error("animation keyframes must use px units"));
    }
    let Some(number) = value
        .get(..unit_start)
        .filter(|number| is_css_number_syntax(number))
    else {
        return Err(type_error(
            "animation keyframe values must be finite numbers",
        ));
    };
    let number = number
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .ok_or_else(|| type_error("animation keyframe values must be finite numbers"))?;
    Ok(number)
}

fn is_css_number_syntax(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let integer_start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let has_integer = index > integer_start;

    let has_fraction = if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == fraction_start {
            return false;
        }
        true
    } else {
        false
    };
    if !has_integer && !has_fraction {
        return false;
    }

    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }

    index == bytes.len()
}

/// Check a keyframe sequence the static sampler does not render: every
/// keyframe is a dictionary whose `composite` is a known operation, offsets
/// that are present lie in [0, 1] in non-decreasing order, and every
/// `easing` is an `<easing-function>`.
fn validate_keyframe_sequence(sequence: &JsObject, context: &mut Context) -> JsResult<()> {
    const COMPOSITE_OR_AUTO: &[&str] = &["replace", "add", "accumulate", "auto"];
    let length = sequence
        .get(js_string!("length"), context)?
        .to_length(context)?;
    let mut previous_offset = 0.0;
    let mut easings = Vec::new();
    for index in 0..length {
        let keyframe = sequence.get(index, context)?;
        if keyframe.is_null_or_undefined() {
            continue;
        }
        let keyframe = keyframe
            .as_object()
            .ok_or_else(|| type_error("each keyframe must be an object"))?;
        // BaseKeyframe members are read in lexicographic order.
        enum_member(
            &keyframe,
            js_string!("composite"),
            COMPOSITE_OR_AUTO,
            context,
        )?;
        easings.extend(string_member(&keyframe, js_string!("easing"), context)?);
        let offset = keyframe.get(js_string!("offset"), context)?;
        if offset.is_null_or_undefined() {
            continue;
        }
        let offset = offset.to_number(context)?;
        if !(previous_offset..=1.0).contains(&offset) {
            return Err(type_error(
                "keyframe offsets must lie in [0, 1] in non-decreasing order",
            ));
        }
        previous_offset = offset;
    }
    if easings
        .iter()
        .all(|easing| is_valid_easing_function(easing))
    {
        Ok(())
    } else {
        Err(type_error("keyframe easing must be an <easing-function>"))
    }
}

/// Parse keyframes into the one sampled property, or `None` when they fall
/// outside the static sampler (null, a keyframe sequence, no property, an
/// unsupported property, or several properties).
fn parse_keyframes(
    value: &JsValue,
    context: &mut Context,
) -> JsResult<Option<(AnimationProperty, f64, f64)>> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let object = value
        .as_object()
        .ok_or_else(|| type_error("animation keyframes must be an object"))?;
    if object.is_array() {
        validate_keyframe_sequence(&object, context)?;
        return Ok(None);
    }
    let keys = object.own_property_keys(context)?;
    let [PropertyKey::String(name)] = keys.as_slice() else {
        return Ok(None);
    };
    let property_name = name.to_std_string_escaped();
    let Ok(property) = AnimationProperty::parse_cssom_name(&property_name) else {
        return Ok(None);
    };
    let values = object.get(JsString::from(property_name), context)?;
    let values = values
        .as_object()
        .filter(|values| values.is_array())
        .ok_or_else(|| type_error("animation property values must be a two-item array"))?;
    let length = values.get(js_string!("length"), context)?.as_number();
    if length != Some(2.0) {
        return Err(type_error(
            "animation property values must contain two keyframes",
        ));
    }
    let from = values.get(js_string!("0"), context)?;
    let to = values.get(js_string!("1"), context)?;
    Ok(Some((
        property,
        parse_pixel_value(&from)?,
        parse_pixel_value(&to)?,
    )))
}

/// Timing from the `options` argument of `Element.animate`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Timing {
    duration_ms: f64,
    delay_ms: f64,
    end_delay_ms: f64,
    iterations: f64,
    iteration_start: f64,
    /// Whether direction, easing, fill or composite differ from the values
    /// the static sampler assumes (`normal`, `linear`, no fill, `replace`).
    shaped: bool,
}

impl Timing {
    /// Whether the static sampler can render the effect: it only models a
    /// single finite linear iteration that starts at once and does not fill.
    fn is_sampleable(self) -> bool {
        self.duration_ms.is_finite()
            && self.delay_ms == 0.0
            && self.end_delay_ms == 0.0
            && self.iterations == 1.0
            && self.iteration_start == 0.0
            && !self.shaped
    }

    /// Effect end time: delays plus the active duration, never negative. The
    /// active duration is zero when either factor is zero, even if the other
    /// is infinite.
    fn end_time_ms(self) -> f64 {
        let active = if self.duration_ms == 0.0 || self.iterations == 0.0 {
            0.0
        } else {
            self.duration_ms * self.iterations
        };
        (self.delay_ms + active + self.end_delay_ms).max(0.0)
    }
}

/// Timing from `options`, converted as the WebIDL union
/// `(unrestricted double or KeyframeAnimationOptions)`: `undefined` and
/// `null` are an empty dictionary, an object is a dictionary, and any other
/// primitive is converted to a number. A dictionary's `duration` is a Number
/// or a value whose string is `"auto"`; absent and `"auto"` mean zero for a
/// keyframe effect.
fn parse_timing(options: Option<&JsValue>, context: &mut Context) -> JsResult<Timing> {
    let options = options.cloned().unwrap_or_default();
    let mut timing = Timing {
        duration_ms: 0.0,
        delay_ms: 0.0,
        end_delay_ms: 0.0,
        iterations: 1.0,
        iteration_start: 0.0,
        shaped: false,
    };
    let mut easing = None;
    let duration = if options.is_null_or_undefined() {
        JsValue::undefined()
    } else if let Some(dictionary) = options.as_object() {
        let (duration, member_easing) = parse_timing_dictionary(&dictionary, &mut timing, context)?;
        easing = member_easing;
        duration
    } else {
        JsValue::from(options.to_number(context)?)
    };
    if timing.iteration_start < 0.0 {
        return Err(type_error("animation iterationStart must be non-negative"));
    }
    if timing.iterations.is_nan() || timing.iterations < 0.0 {
        return Err(type_error("animation iterations must be non-negative"));
    }
    // `(unrestricted double or DOMString)`: only a Number selects the double;
    // every other value converts to a string, and only "auto" is valid.
    timing.duration_ms = if duration.is_undefined() {
        0.0
    } else if let Some(duration) = duration.as_number() {
        duration
    } else if duration.to_string(context)?.to_std_string_escaped() == "auto" {
        0.0
    } else {
        return Err(type_error(
            "animation duration must be a number or \"auto\"",
        ));
    };
    if timing.duration_ms.is_nan() || timing.duration_ms < 0.0 {
        return Err(type_error("animation duration must be non-negative"));
    }
    if easing
        .as_deref()
        .is_some_and(|easing| !is_valid_easing_function(easing))
    {
        return Err(type_error("animation easing must be an <easing-function>"));
    }
    timing.shaped |= easing.is_some_and(|easing| !easing.trim().eq_ignore_ascii_case("linear"));
    Ok(timing)
}

/// Read the `KeyframeAnimationOptions` members this model knows: the
/// `EffectTiming` members, then `KeyframeEffectOptions`'s `composite` and
/// `iterationComposite`, each group in lexicographic order. Returns the raw
/// `duration` and `easing` values for the caller to validate.
fn parse_timing_dictionary(
    dictionary: &JsObject,
    timing: &mut Timing,
    context: &mut Context,
) -> JsResult<(JsValue, Option<String>)> {
    const DIRECTIONS: &[&str] = &["normal", "reverse", "alternate", "alternate-reverse"];
    const FILLS: &[&str] = &["none", "forwards", "backwards", "both", "auto"];
    const COMPOSITES: &[&str] = &["replace", "add", "accumulate"];
    const ITERATION_COMPOSITES: &[&str] = &["replace", "accumulate"];
    timing.delay_ms = finite_member(dictionary, js_string!("delay"), 0.0, context)?;
    let direction = enum_member(dictionary, js_string!("direction"), DIRECTIONS, context)?;
    let duration = dictionary.get(js_string!("duration"), context)?;
    let easing = string_member(dictionary, js_string!("easing"), context)?;
    timing.end_delay_ms = finite_member(dictionary, js_string!("endDelay"), 0.0, context)?;
    let fill = enum_member(dictionary, js_string!("fill"), FILLS, context)?;
    timing.iteration_start = finite_member(dictionary, js_string!("iterationStart"), 0.0, context)?;
    let iterations = dictionary.get(js_string!("iterations"), context)?;
    if !iterations.is_undefined() {
        timing.iterations = iterations.to_number(context)?;
    }
    let composite = enum_member(dictionary, js_string!("composite"), COMPOSITES, context)?;
    enum_member(
        dictionary,
        js_string!("iterationComposite"),
        ITERATION_COMPOSITES,
        context,
    )?;
    timing.shaped = direction.is_some_and(|direction| direction != "normal")
        || fill.is_some_and(|fill| !matches!(fill, "none" | "auto"))
        || composite.is_some_and(|composite| composite != "replace");
    Ok((duration, easing))
}

/// A WebIDL `DOMString` dictionary member, or `None` when absent.
fn string_member(
    dictionary: &JsObject,
    name: JsString,
    context: &mut Context,
) -> JsResult<Option<String>> {
    let value = dictionary.get(name, context)?;
    if value.is_undefined() {
        return Ok(None);
    }
    Ok(Some(value.to_string(context)?.to_std_string_escaped()))
}

/// A WebIDL enumeration dictionary member: absent is `None`, and a string
/// outside `values` throws.
fn enum_member(
    dictionary: &JsObject,
    name: JsString,
    values: &'static [&'static str],
    context: &mut Context,
) -> JsResult<Option<&'static str>> {
    let Some(value) = string_member(dictionary, name, context)? else {
        return Ok(None);
    };
    values
        .iter()
        .find(|candidate| **candidate == value)
        .map(|candidate| Some(*candidate))
        .ok_or_else(|| type_error("animation option is not a valid enumeration value"))
}

/// A WebIDL `double` dictionary member: absent is `default`, and a value
/// that is not finite after conversion throws.
fn finite_member(
    dictionary: &JsObject,
    name: JsString,
    default: f64,
    context: &mut Context,
) -> JsResult<f64> {
    let value = dictionary.get(name, context)?;
    if value.is_undefined() {
        return Ok(default);
    }
    let value = value.to_number(context)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(type_error(
            "animation timing members must be finite numbers",
        ))
    }
}

fn parse_effect(
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<(Option<KeyframePair>, Timing)> {
    let keyframes = args
        .first()
        .ok_or_else(|| type_error("animation keyframes are required"))?;
    let keyframes = parse_keyframes(keyframes, context)?;
    let timing = parse_timing(args.get(1), context)?;
    if keyframes.is_some() && !timing.is_sampleable() {
        return Err(type_error(
            "sampled animations support only one finite linear iteration without delays or fill",
        ));
    }
    let keyframes = keyframes
        .map(|(property, from, to)| KeyframePair::new(property, from, to, timing.duration_ms))
        .transpose()
        .map_err(|_| type_error("animation keyframe values must be finite"))?;
    Ok((keyframes, timing))
}

fn animation_id(this: &JsValue) -> JsResult<usize> {
    this.as_object()
        .and_then(|object| object.downcast_ref::<AnimationData>().map(|data| data.id))
        .ok_or_else(|| type_error("'this' is not an Animation"))
}

/// `Element.animate(keyframes, options)`. Only one-property pixel keyframes
/// are sampled, and only while paused; any other keyframes create an effect
/// that keeps timing (`ready`, `finished`) but never styles its target.
pub(crate) fn element_animate(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let target = this_element(this, context)?;
    let (keyframes, timing) = parse_effect(args, context)?;
    let id = with_state(context, |state| {
        let id = state.animations.len();
        let start_time_ms = state.event_loop.now;
        state.animations.push(AnimationEffect {
            target,
            keyframes,
            start_time_ms,
            end_time_ms: timing.end_time_ms(),
            current_time_ms: None,
            paused: false,
            finish_task: None,
        });
        id
    })?;
    refresh_animation_styles(context)?;
    let prototype = protos(context).animation.clone();
    let data = AnimationData {
        id,
        ready: None,
        finished: None,
        finish_resolve: None,
    };
    Ok(JsObject::from_proto_and_data(Some(prototype), data).into())
}

fn this_animation(this: &JsValue) -> JsResult<JsObject> {
    this.as_object()
        .filter(|object| object.is::<AnimationData>())
        .ok_or_else(|| type_error("'this' is not an Animation"))
}

/// `Animation.ready`: play-pending ends at once, so the promise is already
/// resolved with the animation.
fn ready(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let animation = this_animation(this)?;
    if let Some(ready) = animation_data(&animation).ready.clone() {
        return Ok(ready.into());
    }
    let ready: JsObject = JsPromise::resolve(animation.clone(), context)?.into();
    animation_data_mut(&animation).ready = Some(ready.clone());
    Ok(ready.into())
}

/// `Animation.finished`: resolves with the animation once the virtual clock
/// reaches the effect end time; a paused animation never finishes. The
/// finishing task is queued on first access so that long animations nobody
/// waits on do not hold the event loop open; pausing cancels it and seeking
/// reschedules it.
fn finished(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let animation = this_animation(this)?;
    if let Some(finished) = animation_data(&animation).finished.clone() {
        return Ok(finished.into());
    }
    let (promise, resolvers) = JsPromise::new_pending(context);
    let finished: JsObject = promise.into();
    {
        let mut data = animation_data_mut(&animation);
        data.finished = Some(finished.clone());
        data.finish_resolve = Some(resolvers.resolve);
    }
    schedule_finish(&animation, context)?;
    Ok(finished.into())
}

/// Replace any queued finishing task with one due at the effect end time,
/// or resolve `finished` at once when that time has passed. Does nothing
/// once `finished` is resolved or before anyone has asked for it.
fn schedule_finish(animation: &JsObject, context: &mut Context) -> JsResult<()> {
    if animation_data(animation).finish_resolve.is_none() {
        return Ok(());
    }
    let id = animation_data(animation).id;
    let remaining = with_state(context, |state| {
        let effect = state.animations.get_mut(id)?;
        if let Some(token) = effect.finish_task.take() {
            state.event_loop.cancel_native_task(token);
        }
        Some(
            (!effect.paused)
                .then_some(effect.start_time_ms + effect.end_time_ms - state.event_loop.now)
                .filter(|remaining| remaining.is_finite()),
        )
    })?
    .ok_or_else(|| type_error("Animation is no longer available"))?;
    match remaining {
        None => {}
        Some(remaining) if remaining <= 0.0 => {
            resolve_finished(animation, context)?;
        }
        Some(remaining) => {
            let animation = animation.clone();
            let job = NativeJob::new(move |context| resolve_finished(&animation, context));
            with_state(context, |state| {
                let token = state
                    .event_loop
                    .queue_cancellable_native_task(remaining, job);
                state.animations[id].finish_task = Some(token);
            })?;
        }
    }
    Ok(())
}

/// Resolve `finished` with the animation, at most once.
fn resolve_finished(animation: &JsObject, context: &mut Context) -> JsResult<JsValue> {
    let resolve = animation_data_mut(animation).finish_resolve.take();
    resolve.map_or(Ok(JsValue::undefined()), |resolve| {
        resolve.call(&JsValue::undefined(), &[animation.clone().into()], context)
    })
}

fn animation_data(object: &JsObject) -> boa_engine::gc::GcRef<'_, AnimationData> {
    object
        .downcast_ref::<AnimationData>()
        .expect("checked by this_animation")
}

fn animation_data_mut(object: &JsObject) -> boa_engine::gc::GcRefMut<'_, AnimationData> {
    object
        .downcast_mut::<AnimationData>()
        .expect("checked by this_animation")
}

/// Run `update` on a sampled effect, refusing effects the static sampler
/// cannot render so that pausing or seeking them fails loudly.
fn update_sampled_effect(
    id: usize,
    context: &mut Context,
    update: impl FnOnce(&mut AnimationEffect),
) -> JsResult<()> {
    let found = with_state(context, |state| {
        let effect = state.animations.get_mut(id)?;
        if effect.keyframes.is_none() {
            return Some(false);
        }
        update(effect);
        Some(true)
    })?;
    match found {
        None => Err(type_error("Animation is no longer available")),
        Some(false) => Err(type_error(
            "only single-property pixel animations can be paused or seeked",
        )),
        Some(true) => refresh_animation_styles(context),
    }
}

fn pause(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = animation_id(this)?;
    update_sampled_effect(id, context, |effect| {
        effect.paused = true;
        effect.current_time_ms.get_or_insert(0.0);
    })?;
    with_state(context, |state| {
        let token = state.animations.get_mut(id)?.finish_task.take()?;
        state.event_loop.cancel_native_task(token);
        Some(())
    })?;
    Ok(JsValue::undefined())
}

fn current_time(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = animation_id(this)?;
    let current_time = with_state(context, |state| {
        state
            .animations
            .get(id)
            .map(|effect| effect.current_time_ms)
    })?
    .ok_or_else(|| type_error("Animation is no longer available"))?;
    Ok(current_time.map_or_else(JsValue::null, JsValue::from))
}

fn set_current_time(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = animation_id(this)?;
    let current_time = args
        .first()
        .filter(|value| value.is_number())
        .and_then(JsValue::as_number)
        .filter(|time| time.is_finite())
        .ok_or_else(|| type_error("currentTime must be a finite number"))?;
    let now = with_state(context, |state| state.event_loop.now)?;
    update_sampled_effect(id, context, |effect| {
        effect.current_time_ms = Some(current_time);
        if !effect.paused {
            // Seeking moves the start time so that the effect reaches
            // `current_time` now.
            effect.start_time_ms = now - current_time;
        }
    })?;
    schedule_finish(&this_animation(this)?, context)?;
    Ok(JsValue::undefined())
}

fn refresh_animation_styles(context: &mut Context) -> JsResult<()> {
    with_state(context, |state| {
        let mut target_order = Vec::new();
        let mut targets = HashSet::new();
        let mut declarations = HashMap::<usize, String>::new();
        for effect in &state.animations {
            if targets.insert(effect.target) {
                target_order.push(effect.target);
            }
            if !effect.paused {
                continue;
            }
            let (Some(keyframes), Some(current_time)) = (effect.keyframes, effect.current_time_ms)
            else {
                continue;
            };
            let Some(value) = keyframes.sample(current_time) else {
                continue;
            };
            let declarations = declarations.entry(effect.target).or_default();
            if !declarations.is_empty() {
                declarations.push(' ');
            }
            declarations.push_str(keyframes.property.css_name());
            declarations.push_str(": ");
            declarations.push_str(&value.to_string());
            declarations.push_str("px;");
        }

        let document = state.host.document_mut();
        for target in target_order {
            let style = declarations.remove(&target).map(Into::into);
            document.set_element_animation_style(target, style);
        }
        state.dirty = true;
    })?;
    Ok(())
}

pub(crate) const ANIMATION_MEMBERS: Members = Members {
    getters: &[("ready", ready), ("finished", finished)],
    accessors: &[("currentTime", current_time, set_current_time)],
    methods: &[("pause", 0, pause)],
};

#[cfg(test)]
mod tests;
