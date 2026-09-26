//! `DOMRect` / `DOMRectReadOnly` (Geometry Interfaces Module Level 1 §3),
//! `Element.getBoundingClientRect`'s return type, and the CSSOM View box
//! metrics (`offset*`, `client*`, `scroll*`) computed from the host's
//! [`BoxGeometry`].
//!
//! Spec refs: <https://drafts.fxtf.org/geometry/#DOMRect>,
//! <https://drafts.csswg.org/cssom-view/#extension-to-the-element-interface>,
//! <https://drafts.csswg.org/cssom-view/#extensions-to-the-htmlelement-interface>

use std::cell::Cell;

use boa_engine::object::{JsObject, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsResult, JsValue, Trace, js_string};

use super::host::{BoxGeometry, DomRect, PositionKind};
use super::interfaces::{HTML_NS, Members, protos, wrap_optional};
use super::node::first_element_child;
use super::style::ensure_flushed;
use super::webidl::{host_failure, this_element, with_state};

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
/// optional-with-default: a genuinely missing trailing argument, or one
/// explicitly passed as `undefined`, takes `default` directly; anything
/// else runs `ToNumber`, which can produce `NaN`, exactly as
/// `unrestricted` allows).
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

// ---- CSSOM View box metrics ------------------------------------------------

/// Geometry of `index`'s principal box after bringing layout up to date, or
/// `None` when it has no box.
pub(crate) fn box_geometry(context: &mut Context, index: usize) -> JsResult<Option<BoxGeometry>> {
    ensure_flushed(context)?;
    let geometry = with_state(context, |s| s.host.box_geometry(index))?;
    geometry.map_err(|error| host_failure(context, error))
}

/// A CSSOM View `long` metric: every `offset*`, `client*`, and
/// `scrollWidth`/`scrollHeight` attribute is declared `long`, so the
/// layout value is rounded to the nearest integer (half away from zero).
/// The saturating cast also turns a `-0.0` into `0`.
fn long(value: f64) -> JsValue {
    JsValue::from(value.round() as i32)
}

/// The document's root element and its body element (the `body` child of
/// the `html` document element, exactly as `document.body` finds it).
fn root_and_body(context: &mut Context) -> JsResult<(Option<usize>, Option<usize>)> {
    with_state(context, |s| {
        let doc = s.host.document();
        let document = doc.root_index();
        let root = first_element_child(doc, document, None);
        let body = first_element_child(doc, document, Some("html"))
            .and_then(|html| first_element_child(doc, html, Some("body")));
        (root, body)
    })
}

/// `index`'s parent when that parent is an element, and whether `index`
/// itself is an HTML `td`, `th`, or `table`.
fn parent_and_table_part(context: &mut Context, index: usize) -> JsResult<(Option<usize>, bool)> {
    with_state(context, |s| {
        let doc = s.host.document();
        let parent = doc
            .parent_of(index)
            .filter(|&p| doc.get_node(p).and_then(|n| n.tag_name()).is_some());
        let table_part = doc.element_namespace_uri(index) == Some(HTML_NS)
            && doc
                .get_node(index)
                .and_then(|n| n.tag_name())
                .is_some_and(|t| matches!(t, "td" | "th" | "table"));
        (parent, table_part)
    })
}

/// `HTMLElement.offsetParent` (CSSOM View §7). No fixed-position
/// containing block other than the viewport is modeled (transforms,
/// filters, and containment are not exposed by the host), so a `fixed`
/// element always gets `null`, and an ancestor qualifies when its
/// `position` is not `static`. An ancestor with no box reads as `static`.
fn offset_parent_of(context: &mut Context, index: usize) -> JsResult<Option<usize>> {
    let Some(geometry) = box_geometry(context, index)? else {
        return Ok(None);
    };
    let (root, body) = root_and_body(context)?;
    if Some(index) == root || Some(index) == body || geometry.position == PositionKind::Fixed {
        return Ok(None);
    }
    let element_is_static = geometry.position == PositionKind::Static;
    let mut ancestor = parent_and_table_part(context, index)?.0;
    while let Some(candidate) = ancestor {
        let (parent, table_part) = parent_and_table_part(context, candidate)?;
        let positioned =
            box_geometry(context, candidate)?.is_some_and(|g| g.position != PositionKind::Static);
        if positioned || Some(candidate) == body || (element_is_static && table_part) {
            return Ok(Some(candidate));
        }
        ancestor = parent;
    }
    Ok(None)
}

fn offset_parent(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let parent = offset_parent_of(context, index)?;
    wrap_optional(context, parent)
}

/// `offsetTop`/`offsetLeft` (CSSOM View §7) along the axis `edge` picks.
///
/// Zero for the body element or an element without a box; the border edge
/// relative to the initial containing block when there is no offsetParent;
/// otherwise the border edge minus the offsetParent's padding edge. When
/// the offsetParent is the body element, the border edge is returned
/// as-is (relative to the initial containing block) instead of being made
/// relative to the body's padding edge: that is what engines report, and
/// what content measuring against a `static` body expects.
fn offset_coordinate(
    this: &JsValue,
    context: &mut Context,
    edge: fn(&DomRect) -> f64,
) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let Some(geometry) = box_geometry(context, index)? else {
        return Ok(long(0.0));
    };
    let (_, body) = root_and_body(context)?;
    if Some(index) == body {
        return Ok(long(0.0));
    }
    let origin = match offset_parent_of(context, index)? {
        Some(parent) if Some(parent) != body => {
            let parent_geometry = box_geometry(context, parent)?;
            parent_geometry.map_or(0.0, |g| edge(&g.padding_box))
        }
        _ => 0.0,
    };
    Ok(long(edge(&geometry.border_box) - origin))
}

fn offset_top(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    offset_coordinate(this, context, |r| r.top)
}

fn offset_left(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    offset_coordinate(this, context, |r| r.left)
}

/// A metric read straight off the element's box, `0` when it has no box.
fn box_metric(
    this: &JsValue,
    context: &mut Context,
    metric: fn(&BoxGeometry) -> f64,
) -> JsResult<JsValue> {
    let index = this_element(this, context)?;
    let geometry = box_geometry(context, index)?;
    Ok(long(geometry.map_or(0.0, |g| metric(&g))))
}

/// `offsetWidth`/`offsetHeight` (CSSOM View §7): the border box size.
fn offset_width(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    box_metric(this, context, |g| g.border_box.width)
}

fn offset_height(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    box_metric(this, context, |g| g.border_box.height)
}

/// `clientTop`/`clientLeft` (CSSOM View §6): the top/left border width,
/// the distance between the border and padding edges (no scrollbars are
/// rendered).
fn client_top(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    box_metric(this, context, |g| g.padding_box.top - g.border_box.top)
}

fn client_left(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    box_metric(this, context, |g| g.padding_box.left - g.border_box.left)
}

/// `clientWidth`/`clientHeight` (CSSOM View §6): the padding box size.
fn client_width(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    box_metric(this, context, |g| g.padding_box.width)
}

fn client_height(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    box_metric(this, context, |g| g.padding_box.height)
}

/// `scrollWidth`/`scrollHeight` (CSSOM View §6): the scrolling area size.
fn scroll_width(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    box_metric(this, context, |g| g.scroll_width)
}

fn scroll_height(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    box_metric(this, context, |g| g.scroll_height)
}

/// `scrollTop`/`scrollLeft` (CSSOM View §6, `unrestricted double`). This
/// runtime keeps no scroll state, so every element stays at its origin.
fn scroll_position(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    this_element(this, context)?;
    Ok(JsValue::from(0))
}

/// The `scrollTop`/`scrollLeft` setters: the argument still goes through
/// the `unrestricted double` conversion (running any `valueOf`), then the
/// scroll request is dropped because nothing scrolls.
fn set_scroll_position(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    this_element(this, context)?;
    args.first()
        .cloned()
        .unwrap_or_default()
        .to_number(context)?;
    Ok(JsValue::undefined())
}

pub(crate) const HTML_ELEMENT_OFFSET_MEMBERS: Members = Members {
    getters: &[
        ("offsetParent", offset_parent),
        ("offsetTop", offset_top),
        ("offsetLeft", offset_left),
        ("offsetWidth", offset_width),
        ("offsetHeight", offset_height),
    ],
    accessors: &[],
    methods: &[],
};

pub(crate) const ELEMENT_METRICS_MEMBERS: Members = Members {
    getters: &[
        ("clientTop", client_top),
        ("clientLeft", client_left),
        ("clientWidth", client_width),
        ("clientHeight", client_height),
        ("scrollWidth", scroll_width),
        ("scrollHeight", scroll_height),
    ],
    accessors: &[
        ("scrollTop", scroll_position, set_scroll_position),
        ("scrollLeft", scroll_position, set_scroll_position),
    ],
    methods: &[],
};

#[cfg(test)]
mod tests;
