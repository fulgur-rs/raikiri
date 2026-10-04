use std::collections::{HashMap, HashSet};

use boa_engine::object::JsObject;
use boa_engine::property::PropertyKey;
use boa_engine::{
    Context, Finalize, JsData, JsNativeError, JsResult, JsString, JsValue, Trace, js_string,
};

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
    pub(crate) keyframes: KeyframePair,
    pub(crate) current_time_ms: Option<f64>,
    pub(crate) paused: bool,
}

/// Native data behind an `Animation` instance.
#[derive(Debug, Trace, Finalize, JsData)]
pub(crate) struct AnimationData {
    #[unsafe_ignore_trace]
    id: usize,
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

fn parse_keyframes(
    value: &JsValue,
    context: &mut Context,
) -> JsResult<(AnimationProperty, f64, f64)> {
    let object = value
        .as_object()
        .ok_or_else(|| type_error("animation keyframes must be an object"))?;
    let keys = object.own_property_keys(context)?;
    let [PropertyKey::String(name)] = keys.as_slice() else {
        return Err(type_error(
            "exactly one supported animation property is required",
        ));
    };
    let property_name = name.to_std_string_escaped();
    let property = AnimationProperty::parse_cssom_name(&property_name)
        .map_err(|_| type_error("unsupported animation property"))?;
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
    Ok((property, parse_pixel_value(&from)?, parse_pixel_value(&to)?))
}

fn parse_effect(args: &[JsValue], context: &mut Context) -> JsResult<KeyframePair> {
    let keyframes = args
        .first()
        .ok_or_else(|| type_error("animation keyframes are required"))?;
    let (property, from, to) = parse_keyframes(keyframes, context)?;
    let duration = args
        .get(1)
        .filter(|value| value.is_number())
        .and_then(JsValue::as_number)
        .ok_or_else(|| type_error("animation duration must be a number"))?;
    KeyframePair::new(property, from, to, duration).map_err(|_| {
        type_error("animation values and duration must be finite, with non-negative duration")
    })
}

fn animation_id(this: &JsValue) -> JsResult<usize> {
    this.as_object()
        .and_then(|object| object.downcast_ref::<AnimationData>().map(|data| data.id))
        .ok_or_else(|| type_error("'this' is not an Animation"))
}

/// `Element.animate(keyframes, duration)` for the pinned CSS Text fixtures.
pub(crate) fn element_animate(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let target = this_element(this, context)?;
    let keyframes = parse_effect(args, context)?;
    let id = with_state(context, |state| {
        let id = state.animations.len();
        state.animations.push(AnimationEffect {
            target,
            keyframes,
            current_time_ms: None,
            paused: false,
        });
        id
    })?;
    refresh_animation_styles(context)?;
    let prototype = protos(context).animation.clone();
    Ok(JsObject::from_proto_and_data(Some(prototype), AnimationData { id }).into())
}

fn pause(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = animation_id(this)?;
    let found = with_state(context, |state| {
        let Some(effect) = state.animations.get_mut(id) else {
            return false;
        };
        effect.paused = true;
        effect.current_time_ms.get_or_insert(0.0);
        true
    })?;
    if !found {
        return Err(type_error("Animation is no longer available"));
    }
    refresh_animation_styles(context)?;
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
    let found = with_state(context, |state| {
        let Some(effect) = state.animations.get_mut(id) else {
            return false;
        };
        effect.current_time_ms = Some(current_time);
        true
    })?;
    if !found {
        return Err(type_error("Animation is no longer available"));
    }
    refresh_animation_styles(context)?;
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
            let Some(current_time) = effect.current_time_ms else {
                continue;
            };
            let Some(value) = effect.keyframes.sample(current_time) else {
                continue;
            };
            let declarations = declarations.entry(effect.target).or_default();
            if !declarations.is_empty() {
                declarations.push(' ');
            }
            declarations.push_str(effect.keyframes.property.css_name());
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
    getters: &[],
    accessors: &[("currentTime", current_time, set_current_time)],
    methods: &[("pause", 0, pause)],
};

#[cfg(test)]
mod tests;
