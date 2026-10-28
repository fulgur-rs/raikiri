use std::collections::{HashMap, HashSet};

use boa_engine::builtins::object::OrdinaryObject;
use boa_engine::job::NativeJob;
use boa_engine::object::JsObject;
use boa_engine::object::builtins::{JsFunction, JsPromise};
use boa_engine::property::PropertyKey;
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsSymbol, JsValue, Trace,
    js_string,
};

use raikiri_style::is_valid_easing_function;
use raikiri_style::property::is_animatable_property_name;

use super::interfaces::{Members, protos};
use super::webidl::{this_element, throw_dom_exception, with_state};

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
    /// Whether the animation has no timeline: it never starts, so its
    /// current time is unresolved and its promises stay pending.
    pub(crate) detached: bool,
    /// The queued task that resolves `finished`, cancelled on pause.
    pub(crate) finish_task: Option<u64>,
}

impl AnimationEffect {
    /// The current time: the held time while paused, unresolved without a
    /// timeline, and otherwise the time elapsed since the start time. Past
    /// the effect end, a naturally finished animation holds at the end, and
    /// one sought beyond the end holds where it was sought.
    fn current_time(&self, now: f64) -> Option<f64> {
        if self.paused {
            return self.current_time_ms;
        }
        if self.detached {
            return None;
        }
        let elapsed = now - self.start_time_ms;
        if elapsed < self.end_time_ms {
            return Some(elapsed);
        }
        Some(
            self.current_time_ms
                .map_or(self.end_time_ms, |sought| sought.max(self.end_time_ms)),
        )
    }
}

/// Native data behind an `Animation` instance.
#[derive(Debug, Trace, Finalize, JsData)]
pub(crate) struct AnimationData {
    target_owner: boa_gc::Gc<super::node_gc::NodeRecord>,
    #[unsafe_ignore_trace]
    id: usize,
    /// The script-visible `Animation.id`.
    #[unsafe_ignore_trace]
    animation_id: String,
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

/// A finite `<number>px` length, or `None` for any other value.
fn parse_pixel_value(value: &str) -> Option<f64> {
    let value = value.trim();
    let unit_start = value.len().checked_sub(2)?;
    if !value.as_bytes()[unit_start..].eq_ignore_ascii_case(b"px") {
        return None;
    }
    value
        .get(..unit_start)
        .filter(|number| is_css_number_syntax(number))?
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
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

const COMPOSITE_OR_AUTO: &[&str] = &["replace", "add", "accumulate", "auto"];

/// The `@@iterator` method of `value`, or `None` when `value` is not an
/// object or has none; a method that is not callable throws.
fn iterator_method(value: &JsValue, context: &mut Context) -> JsResult<Option<JsObject>> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let method = object.get(JsSymbol::iterator(), context)?;
    if method.is_null_or_undefined() {
        return Ok(None);
    }
    method
        .as_callable()
        .map(Some)
        .ok_or_else(|| type_error("@@iterator must be callable"))
}

/// Call `each` on every value `method` yields when iterating `iterable`.
fn for_each_iterated(
    iterable: &JsValue,
    method: &JsObject,
    context: &mut Context,
    mut each: impl FnMut(JsValue, &mut Context) -> JsResult<()>,
) -> JsResult<()> {
    let iterator = method.call(iterable, &[], context)?;
    let iterator = iterator
        .as_object()
        .ok_or_else(|| type_error("iterator must be an object"))?;
    let next = iterator.get(js_string!("next"), context)?;
    let next = next
        .as_callable()
        .ok_or_else(|| type_error("iterator next must be callable"))?;
    loop {
        let result = next.call(&iterator.clone().into(), &[], context)?;
        let result = result
            .as_object()
            .ok_or_else(|| type_error("iterator result must be an object"))?;
        if result.get(js_string!("done"), context)?.to_boolean() {
            return Ok(());
        }
        each(result.get(js_string!("value"), context)?, context)?;
    }
}

/// Check converted keyframe offsets: the non-null ones lie in [0, 1] in
/// non-decreasing order.
fn check_offsets(offsets: &[Option<f64>]) -> JsResult<()> {
    let mut previous = 0.0;
    for &offset in offsets.iter().flatten() {
        if !(previous..=1.0).contains(&offset) {
            return Err(type_error(
                "keyframe offsets must lie in [0, 1] in non-decreasing order",
            ));
        }
        previous = offset;
    }
    Ok(())
}

fn check_easings(easings: &[String]) -> JsResult<()> {
    if easings
        .iter()
        .all(|easing| is_valid_easing_function(easing))
    {
        Ok(())
    } else {
        Err(type_error("keyframe easing must be an <easing-function>"))
    }
}

/// A WebIDL `double?` keyframe offset: `null` is `None`, and a value that
/// is not finite after conversion throws.
fn offset_value(value: &JsValue, context: &mut Context) -> JsResult<Option<f64>> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let offset = value.to_number(context)?;
    if offset.is_finite() {
        Ok(Some(offset))
    } else {
        Err(type_error("keyframe offsets must be finite numbers"))
    }
}

fn dom_string(value: &JsValue, context: &mut Context) -> JsResult<String> {
    Ok(value.to_string(context)?.to_std_string_escaped())
}

/// The converted values of one keyframe property: a single value (whose
/// conversion only matters for its side effects) or a list.
enum PropertyValues {
    Single,
    List(Vec<String>),
}

/// Whether `name` names a keyframe animation property: a custom property
/// name, or the IDL attribute name of a supported CSS property that can be
/// animated (`cssFloat` for `float`, otherwise the camel-cased property
/// name). A not-animatable property such as `will-change` is skipped.
fn is_animatable_idl_name(name: &str) -> bool {
    if let Some(custom) = name.strip_prefix("--") {
        return !custom.is_empty();
    }
    if name == "cssFloat" {
        return true;
    }
    if name == "float" || name.contains('-') {
        return false;
    }
    let mut property = String::with_capacity(name.len() + 4);
    for character in name.chars() {
        if character.is_ascii_uppercase() {
            property.push('-');
        }
        property.push(character.to_ascii_lowercase());
    }
    is_animatable_property_name(&property)
}

/// Convert the animation properties of a keyframe-like object: its own
/// enumerable properties named by an animatable property's IDL name, read
/// in code unit order. Each value is a DOMString, or, when `allow_lists`
/// is set and the value is iterable, a sequence of them.
fn convert_keyframe_properties(
    object: &JsObject,
    allow_lists: bool,
    context: &mut Context,
) -> JsResult<Vec<(String, PropertyValues)>> {
    // EnumerableOwnProperties(object, key), through the object's internal
    // methods rather than the script-replaceable `Object.keys`.
    let mut names = Vec::new();
    for key in object.own_property_keys(context)? {
        let PropertyKey::String(name) = key else {
            continue;
        };
        let enumerable = OrdinaryObject::property_is_enumerable(
            &object.clone().into(),
            &[name.clone().into()],
            context,
        )?;
        let name = name.to_std_string_escaped();
        if enumerable.to_boolean() && is_animatable_idl_name(&name) {
            names.push(name);
        }
    }
    names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let mut properties = Vec::with_capacity(names.len());
    for name in names {
        let value = object.get(JsString::from(name.as_str()), context)?;
        let method = if allow_lists {
            iterator_method(&value, context)?
        } else {
            None
        };
        let values = match method {
            Some(method) => {
                let mut list = Vec::new();
                for_each_iterated(&value, &method, context, |value, context| {
                    list.push(dom_string(&value, context)?);
                    Ok(())
                })?;
                PropertyValues::List(list)
            }
            None => {
                dom_string(&value, context)?;
                PropertyValues::Single
            }
        };
        properties.push((name, values));
    }
    Ok(properties)
}

/// Check a keyframe sequence, which the static sampler does not render.
/// The keyframes come from iterating `sequence` with its `@@iterator`
/// `method`. Each is converted as a `BaseKeyframe` dictionary (whose
/// `composite` must be a known operation) followed by its animation
/// properties; then the offsets must lie in [0, 1] in non-decreasing order
/// and every `easing` must be an `<easing-function>`.
fn validate_keyframe_sequence(
    sequence: &JsValue,
    method: &JsObject,
    context: &mut Context,
) -> JsResult<()> {
    let mut offsets = Vec::new();
    let mut easings = Vec::new();
    for_each_iterated(sequence, method, context, |keyframe, context| {
        if keyframe.is_null_or_undefined() {
            return Ok(());
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
        offsets.push(offset_value(&offset, context)?);
        convert_keyframe_properties(&keyframe, false, context)?;
        Ok(())
    })?;
    check_offsets(&offsets)?;
    check_easings(&easings)
}

/// Call `each` on the values of a `(T or sequence<T>)` dictionary member:
/// nothing when absent, every yielded value for an iterable object, and
/// the value itself otherwise.
fn for_each_member_value(
    dictionary: &JsObject,
    name: JsString,
    context: &mut Context,
    mut each: impl FnMut(JsValue, &mut Context) -> JsResult<()>,
) -> JsResult<()> {
    let value = dictionary.get(name, context)?;
    if value.is_undefined() {
        return Ok(());
    }
    match iterator_method(&value, context)? {
        Some(method) => for_each_iterated(&value, &method, context, each),
        None => each(value, context),
    }
}

fn is_linear_keyword(easing: &str) -> bool {
    easing.trim().eq_ignore_ascii_case("linear")
}

/// Convert property-indexed keyframes: the `BasePropertyIndexedKeyframe`
/// members (every `composite` must be a known operation), then the
/// animation properties; then the non-null offsets must lie in [0, 1] in
/// non-decreasing order and every `easing` must be an `<easing-function>`.
/// Also returns whether the members leave two keyframes as the static
/// sampler models them: replacing, linear, at offsets 0 and 1.
fn convert_property_indexed(
    object: &JsObject,
    context: &mut Context,
) -> JsResult<(bool, Vec<(String, PropertyValues)>)> {
    let mut plain = true;
    for_each_member_value(
        object,
        js_string!("composite"),
        context,
        |value, context| {
            let composite = enum_value(&value, COMPOSITE_OR_AUTO, context)?;
            plain &= matches!(composite, "auto" | "replace");
            Ok(())
        },
    )?;
    let mut easings = Vec::new();
    for_each_member_value(object, js_string!("easing"), context, |value, context| {
        easings.push(dom_string(&value, context)?);
        Ok(())
    })?;
    let mut offsets = Vec::new();
    for_each_member_value(object, js_string!("offset"), context, |value, context| {
        offsets.push(offset_value(&value, context)?);
        Ok(())
    })?;
    let properties = convert_keyframe_properties(object, true, context)?;
    check_offsets(&offsets)?;
    check_easings(&easings)?;
    // Only the first keyframe's easing shapes the interval between two
    // keyframes, and offsets beyond the second keyframe are ignored.
    plain &= easings
        .first()
        .is_none_or(|easing| is_linear_keyword(easing));
    plain &= offsets
        .iter()
        .zip([0.0, 1.0])
        .all(|(offset, expected)| offset.is_none_or(|offset| offset == expected));
    Ok((plain, properties))
}

/// Parse keyframes into the one sampled property, or `None` when they fall
/// outside the static sampler (null, a keyframe sequence, no animation
/// property, an unsupported property, several properties, keyframe
/// metadata other than the defaults, or values other than two px lengths).
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
    // An object with `@@iterator` is a keyframe sequence; any other object
    // holds property-indexed keyframes.
    if let Some(method) = iterator_method(value, context)? {
        validate_keyframe_sequence(value, &method, context)?;
        return Ok(None);
    }
    let (plain, properties) = convert_property_indexed(&object, context)?;
    let [(name, values)] = properties.as_slice() else {
        return Ok(None);
    };
    let Ok(property) = AnimationProperty::parse_cssom_name(name) else {
        return Ok(None);
    };
    if !plain {
        return Ok(None);
    }
    // Any other value shape (a single value, a list that is not exactly two
    // keyframes, or a value other than a finite px length) is still a valid
    // keyframe; the effect just falls outside the static sampler.
    let PropertyValues::List(values) = values else {
        return Ok(None);
    };
    let [from, to] = values.as_slice() else {
        return Ok(None);
    };
    let (Some(from), Some(to)) = (parse_pixel_value(from), parse_pixel_value(to)) else {
        return Ok(None);
    };
    Ok(Some((property, from, to)))
}

/// Timing from the `options` argument of `Element.animate`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Timing {
    duration_ms: f64,
    delay_ms: f64,
    end_delay_ms: f64,
    iterations: f64,
    iteration_start: f64,
    /// Whether direction, easing, fill, composite, pseudo-element or
    /// timeline differ from the values the static sampler assumes
    /// (`normal`, `linear`, no fill, `replace`, the element itself, the
    /// document timeline).
    shaped: bool,
    /// Whether the animation has no timeline, so it never starts.
    detached: bool,
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
/// Also returns the `id` option, empty when absent.
fn parse_timing(options: Option<&JsValue>, context: &mut Context) -> JsResult<(Timing, String)> {
    let options = options.cloned().unwrap_or_default();
    let mut timing = Timing {
        duration_ms: 0.0,
        delay_ms: 0.0,
        end_delay_ms: 0.0,
        iterations: 1.0,
        iteration_start: 0.0,
        shaped: false,
        detached: false,
    };
    let RawOptions {
        duration,
        easing,
        pseudo_element,
        id,
    } = if let Some(dictionary) = options.as_object() {
        parse_timing_dictionary(&dictionary, &mut timing, context)?
    } else {
        let duration = if options.is_null_or_undefined() {
            JsValue::undefined()
        } else {
            JsValue::from(options.to_number(context)?)
        };
        RawOptions {
            duration,
            easing: None,
            pseudo_element: None,
            id: None,
        }
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
    timing.shaped |= easing.is_some_and(|easing| !is_linear_keyword(&easing));
    if pseudo_element.is_some_and(|selector| !is_animatable_pseudo_element(&selector)) {
        return Err(throw_dom_exception(
            context,
            "SyntaxError",
            "animation pseudoElement is not a supported pseudo-element",
        ));
    }
    Ok((timing, id.unwrap_or_default()))
}

/// Raw `KeyframeAnimationOptions` members that are validated after the
/// whole dictionary is converted.
struct RawOptions {
    duration: JsValue,
    easing: Option<String>,
    pseudo_element: Option<String>,
    id: Option<String>,
}

/// Convert the `KeyframeAnimationOptions` dictionary: the `EffectTiming`
/// members, then the `KeyframeEffectOptions` members, then the
/// `KeyframeAnimationOptions` members, each group in lexicographic order.
fn parse_timing_dictionary(
    dictionary: &JsObject,
    timing: &mut Timing,
    context: &mut Context,
) -> JsResult<RawOptions> {
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
    let pseudo_element = dictionary.get(js_string!("pseudoElement"), context)?;
    let pseudo_element = if pseudo_element.is_null_or_undefined() {
        None
    } else {
        Some(pseudo_element.to_string(context)?.to_std_string_escaped())
    };
    let id = string_member(dictionary, js_string!("id"), context)?;
    // No `AnimationTimeline` object exists, so only the default (the
    // document timeline) and `null` (no timeline) are valid.
    let timeline = dictionary.get(js_string!("timeline"), context)?;
    if !timeline.is_null_or_undefined() {
        return Err(type_error(
            "animation timeline must be an AnimationTimeline",
        ));
    }
    timing.detached = timeline.is_null();
    timing.shaped = direction.is_some_and(|direction| direction != "normal")
        || fill.is_some_and(|fill| !matches!(fill, "none" | "auto"))
        || composite.is_some_and(|composite| composite != "replace")
        || pseudo_element.is_some()
        || timing.detached;
    Ok(RawOptions {
        duration,
        easing,
        pseudo_element,
        id,
    })
}

/// Whether `selector` names a pseudo-element an animation can target,
/// including the legacy single-colon spellings.
fn is_animatable_pseudo_element(selector: &str) -> bool {
    const PSEUDO_ELEMENTS: [&str; 5] = ["before", "after", "marker", "first-letter", "first-line"];
    const LEGACY: [&str; 4] = ["before", "after", "first-letter", "first-line"];
    let (names, name) = match selector.strip_prefix("::") {
        Some(name) => (&PSEUDO_ELEMENTS[..], name),
        None => (&LEGACY[..], selector.strip_prefix(':').unwrap_or_default()),
    };
    names
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
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
    let value = dictionary.get(name, context)?;
    if value.is_undefined() {
        return Ok(None);
    }
    enum_value(&value, values, context).map(Some)
}

/// A WebIDL enumeration value: a string outside `values` throws.
fn enum_value(
    value: &JsValue,
    values: &'static [&'static str],
    context: &mut Context,
) -> JsResult<&'static str> {
    let value = value.to_string(context)?.to_std_string_escaped();
    values
        .iter()
        .find(|candidate| **candidate == value)
        .copied()
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
) -> JsResult<(Option<KeyframePair>, Timing, String)> {
    let keyframes = args
        .first()
        .ok_or_else(|| type_error("animation keyframes are required"))?;
    // The options dictionary is converted and validated before the
    // keyframes are processed, as the KeyframeEffect constructor does.
    let (timing, id) = parse_timing(args.get(1), context)?;
    let keyframes = parse_keyframes(keyframes, context)?;
    if keyframes.is_some() && !timing.is_sampleable() {
        return Err(type_error(
            "sampled animations support only one finite linear iteration without delays or fill",
        ));
    }
    let keyframes = keyframes
        .map(|(property, from, to)| KeyframePair::new(property, from, to, timing.duration_ms))
        .transpose()
        .map_err(|_| type_error("animation keyframe values must be finite"))?;
    Ok((keyframes, timing, id))
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
    let (keyframes, timing, animation_id) = parse_effect(args, context)?;
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
            detached: timing.detached,
            finish_task: None,
        });
        id
    })?;
    refresh_animation_styles(context)?;
    let prototype = protos(context).animation.clone();
    let target_owner = with_state(context, |state| state.node_record(target))?;
    let data = AnimationData {
        target_owner,
        id,
        animation_id,
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
/// resolved with the animation; without a timeline it never resolves.
fn ready(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let animation = this_animation(this)?;
    if let Some(ready) = animation_data(&animation).ready.clone() {
        return Ok(ready.into());
    }
    let id = animation_data(&animation).id;
    let detached = with_state(context, |state| {
        state.animations.get(id).map(|effect| effect.detached)
    })?
    .ok_or_else(|| type_error("Animation is no longer available"))?;
    let ready: JsObject = if detached {
        JsPromise::new_pending(context).0.into()
    } else {
        JsPromise::resolve(animation.clone(), context)?.into()
    };
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
/// or resolve `finished` at once when that time has passed. An animation
/// that leaves the finished state, by pausing or seeking back, gets a new
/// pending `finished` promise. Does nothing before anyone has asked for
/// `finished`.
fn schedule_finish(animation: &JsObject, context: &mut Context) -> JsResult<()> {
    if animation_data(animation).finished.is_none() {
        return Ok(());
    }
    let id = animation_data(animation).id;
    let remaining = with_state(context, |state| {
        let effect = state.animations.get_mut(id)?;
        if let Some(token) = effect.finish_task.take() {
            state.event_loop.cancel_native_task(token);
        }
        Some(
            (!effect.paused && !effect.detached)
                .then_some(effect.start_time_ms + effect.end_time_ms - state.event_loop.now)
                .filter(|remaining| remaining.is_finite()),
        )
    })?
    .ok_or_else(|| type_error("Animation is no longer available"))?;
    let finished_now = remaining.is_some_and(|remaining| remaining <= 0.0);
    if animation_data(animation).finish_resolve.is_none() {
        if finished_now {
            return Ok(());
        }
        let (promise, resolvers) = JsPromise::new_pending(context);
        let mut data = animation_data_mut(animation);
        data.finished = Some(promise.into());
        data.finish_resolve = Some(resolvers.resolve);
    }
    match remaining {
        None => {}
        Some(_) if finished_now => {
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
    let now = with_state(context, |state| state.event_loop.now)?;
    update_sampled_effect(id, context, |effect| {
        // Pausing holds the current time the effect has reached.
        effect.current_time_ms = effect.current_time(now);
        effect.paused = true;
    })?;
    schedule_finish(&this_animation(this)?, context)?;
    Ok(JsValue::undefined())
}

fn current_time(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = animation_id(this)?;
    let current_time = with_state(context, |state| {
        let now = state.event_loop.now;
        state
            .animations
            .get(id)
            .map(|effect| effect.current_time(now))
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

/// `Animation.id`: the `id` option, or whatever script assigned later.
fn animation_id_get(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let animation = this_animation(this)?;
    let id = animation_data(&animation).animation_id.clone();
    Ok(JsString::from(id).into())
}

fn animation_id_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let animation = this_animation(this)?;
    let id = dom_string(&args.first().cloned().unwrap_or_default(), context)?;
    animation_data_mut(&animation).animation_id = id;
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
    accessors: &[
        ("currentTime", current_time, set_current_time),
        ("id", animation_id_get, animation_id_set),
    ],
    methods: &[("pause", 0, pause)],
};

#[cfg(test)]
mod tests;
