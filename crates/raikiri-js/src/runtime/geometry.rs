//! `DOMRect` / `DOMRectReadOnly` (Geometry Interfaces Module Level 1 §3),
//! and `Element.getBoundingClientRect`'s return type.
//!
//! Spec ref: <https://drafts.fxtf.org/geometry/#DOMRect>

use std::cell::Cell;

use boa_engine::object::{JsObject, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsResult, JsValue, Trace, js_string};

use super::host::DomRect;
use super::interfaces::{Members, protos};

/// Native data behind a `DOMRect`/`DOMRectReadOnly` instance. `x`/`y`/
/// `width`/`height` are the only stored fields (the spec's own model);
/// `top`/`right`/`bottom`/`left` are always computed from them. `Cell`
/// gives interior mutability for `DOMRect`'s writable accessors;
/// `DOMRectReadOnly` shares the exact same storage and getters, since
/// nothing distinguishes the two at the data level -- only which accessors
/// each interface's own prototype installs.
#[derive(Debug, Trace, Finalize, JsData)]
pub(crate) struct DomRectData {
    #[unsafe_ignore_trace]
    x: Cell<f64>,
    #[unsafe_ignore_trace]
    y: Cell<f64>,
    #[unsafe_ignore_trace]
    width: Cell<f64>,
    #[unsafe_ignore_trace]
    height: Cell<f64>,
}

fn with_data<T>(this: &JsValue, f: impl FnOnce(&DomRectData) -> T) -> JsResult<T> {
    this.as_object()
        .and_then(|o| o.downcast_ref::<DomRectData>().map(|d| f(&d)))
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a DOMRect")
                .into()
        })
}

/// `unrestricted double` argument conversion with a default (WebIDL
/// optional-with-default: a genuinely missing trailing argument gets the
/// default directly; one explicitly passed -- even as `undefined` -- still
/// runs `ToNumber`, which can produce `NaN`, exactly as `unrestricted`
/// allows).
fn arg_or(args: &[JsValue], i: usize, default: f64, context: &mut Context) -> JsResult<f64> {
    match args.get(i) {
        None => Ok(default),
        Some(v) if v.is_undefined() => Ok(default),
        Some(v) => v.to_number(context),
    }
}

fn rect_from_args(args: &[JsValue], context: &mut Context) -> JsResult<DomRectData> {
    Ok(DomRectData {
        x: Cell::new(arg_or(args, 0, 0.0, context)?),
        y: Cell::new(arg_or(args, 1, 0.0, context)?),
        width: Cell::new(arg_or(args, 2, 0.0, context)?),
        height: Cell::new(arg_or(args, 3, 0.0, context)?),
    })
}

/// Both constructors are called via the same native-function convention:
/// `this` is the `new.target` value under `[[Construct]]`, or `undefined`
/// under a plain `[[Call]]` -- so `this.is_undefined()` is exactly WebIDL's
/// "If NewTarget is undefined, throw a TypeError" constructor check.
fn require_new(this: &JsValue) -> JsResult<()> {
    if this.is_undefined() {
        return Err(JsNativeError::typ()
            .with_message("Constructor requires 'new'")
            .into());
    }
    Ok(())
}

pub(crate) fn dom_rect_read_only_constructor(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_new(this)?;
    let data = rect_from_args(args, context)?;
    let proto = protos(context).dom_rect_read_only.clone();
    Ok(JsObject::from_proto_and_data(Some(proto), data).into())
}

pub(crate) fn dom_rect_constructor(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_new(this)?;
    let data = rect_from_args(args, context)?;
    let proto = protos(context).dom_rect.clone();
    Ok(JsObject::from_proto_and_data(Some(proto), data).into())
}

/// A new `DOMRect` for `r`, for [`super::style::get_bounding_client_rect`].
pub(crate) fn new_dom_rect(context: &mut Context, r: DomRect) -> JsResult<JsObject> {
    let data = DomRectData {
        x: Cell::new(r.left),
        y: Cell::new(r.top),
        width: Cell::new(r.width),
        height: Cell::new(r.height),
    };
    let proto = protos(context).dom_rect.clone();
    Ok(JsObject::from_proto_and_data(Some(proto), data))
}

/// NaN-propagating minimum (the spec's `top`/`left`, and `toJSON`, define
/// these in terms of ordinary `min`/`max` over possibly-`NaN` numbers --
/// `unrestricted double` allows `NaN` -- which propagate `NaN` the way
/// `Math.min`/`Math.max` do, unlike Rust's IEEE `f64::min`/`max`, which
/// return the non-`NaN` operand instead).
fn nan_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

fn nan_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

fn get_x(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_data(this, |d| JsValue::from(d.x.get()))
}
fn get_y(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_data(this, |d| JsValue::from(d.y.get()))
}
fn get_width(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_data(this, |d| JsValue::from(d.width.get()))
}
fn get_height(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_data(this, |d| JsValue::from(d.height.get()))
}

fn get_top(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_data(this, |d| {
        JsValue::from(nan_min(d.y.get(), d.y.get() + d.height.get()))
    })
}
fn get_right(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_data(this, |d| {
        JsValue::from(nan_max(d.x.get(), d.x.get() + d.width.get()))
    })
}
fn get_bottom(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_data(this, |d| {
        JsValue::from(nan_max(d.y.get(), d.y.get() + d.height.get()))
    })
}
fn get_left(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_data(this, |d| {
        JsValue::from(nan_min(d.x.get(), d.x.get() + d.width.get()))
    })
}

fn set_x(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = args
        .first()
        .cloned()
        .unwrap_or_default()
        .to_number(context)?;
    with_data(this, |d| d.x.set(value))?;
    Ok(JsValue::undefined())
}
fn set_y(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = args
        .first()
        .cloned()
        .unwrap_or_default()
        .to_number(context)?;
    with_data(this, |d| d.y.set(value))?;
    Ok(JsValue::undefined())
}
fn set_width(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = args
        .first()
        .cloned()
        .unwrap_or_default()
        .to_number(context)?;
    with_data(this, |d| d.width.set(value))?;
    Ok(JsValue::undefined())
}
fn set_height(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = args
        .first()
        .cloned()
        .unwrap_or_default()
        .to_number(context)?;
    with_data(this, |d| d.height.set(value))?;
    Ok(JsValue::undefined())
}

fn to_json(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (x, y, width, height) = with_data(this, |d| {
        (d.x.get(), d.y.get(), d.width.get(), d.height.get())
    })?;
    let top = nan_min(y, y + height);
    let left = nan_min(x, x + width);
    let right = nan_max(x, x + width);
    let bottom = nan_max(y, y + height);
    Ok(ObjectInitializer::new(context)
        .property(js_string!("x"), x, Attribute::all())
        .property(js_string!("y"), y, Attribute::all())
        .property(js_string!("width"), width, Attribute::all())
        .property(js_string!("height"), height, Attribute::all())
        .property(js_string!("top"), top, Attribute::all())
        .property(js_string!("right"), right, Attribute::all())
        .property(js_string!("bottom"), bottom, Attribute::all())
        .property(js_string!("left"), left, Attribute::all())
        .build()
        .into())
}

pub(crate) const DOM_RECT_READ_ONLY_MEMBERS: Members = Members {
    getters: &[
        ("x", get_x),
        ("y", get_y),
        ("width", get_width),
        ("height", get_height),
        ("top", get_top),
        ("right", get_right),
        ("bottom", get_bottom),
        ("left", get_left),
    ],
    accessors: &[],
    methods: &[("toJSON", 0, to_json)],
};

pub(crate) const DOM_RECT_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[
        ("x", get_x, set_x),
        ("y", get_y, set_y),
        ("width", get_width, set_width),
        ("height", get_height, set_height),
    ],
    methods: &[],
};

#[cfg(test)]
mod tests;
