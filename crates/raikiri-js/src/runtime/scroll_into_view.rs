//! `Element.scrollIntoView` (CSSOM View §6.1). Only the viewport keeps a
//! scroll offset in this runtime (see [`super::window`]), so the viewport is
//! the one scrolling box the element is scrolled into; element scroll
//! containers keep their origin. `scroll-margin` and `scroll-padding` are
//! not applied.
//!
//! Spec: <https://drafts.csswg.org/cssom-view/#dom-element-scrollintoview>

use boa_engine::object::ObjectInitializer;
use boa_engine::object::builtins::JsPromise;
use boa_engine::property::Attribute;
use boa_engine::{Context, JsNativeError, JsResult, JsString, JsValue, js_string};

use super::geometry::box_geometry;
use super::host::PositionKind;
use super::node::first_element_child;
use super::style::read_computed;
use super::webidl::{this_element, with_state};
use super::window::{clamp_viewport_offset, scroll_behavior};

/// A `ScrollLogicalPosition` value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Position {
    Start,
    Center,
    End,
    Nearest,
}

/// The `ScrollIntoViewOptions` members that steer the scroll.
#[derive(Clone, Copy, Debug)]
struct Options {
    block: Position,
    inline: Position,
    /// `container: "nearest"`: scroll only the nearest scroll container.
    nearest_container: bool,
}

/// Read a `ScrollLogicalPosition` member: missing or `undefined` takes
/// `default`; any other value must be one of the four enum strings.
fn position(
    options: &boa_engine::JsObject,
    name: &str,
    default: Position,
    context: &mut Context,
) -> JsResult<Position> {
    let value = options.get(JsString::from(name), context)?;
    if value.is_undefined() {
        return Ok(default);
    }
    match value.to_string(context)?.to_std_string_escaped().as_str() {
        "start" => Ok(Position::Start),
        "center" => Ok(Position::Center),
        "end" => Ok(Position::End),
        "nearest" => Ok(Position::Nearest),
        _ => Err(JsNativeError::typ()
            .with_message(format!("{name} is not a valid ScrollLogicalPosition"))
            .into()),
    }
}

/// Convert the `(boolean or ScrollIntoViewOptions)` argument. `undefined`,
/// `null` and objects are the dictionary, whose members are read in WebIDL
/// order (inherited `behavior` first, then `block`, `container`, `inline`);
/// any other value converts to a boolean, where `false` aligns the block
/// end.
fn options(arg: Option<&JsValue>, context: &mut Context) -> JsResult<Options> {
    let mut options = Options {
        block: Position::Start,
        inline: Position::Nearest,
        nearest_container: false,
    };
    let Some(arg) = arg.filter(|arg| !arg.is_null_or_undefined()) else {
        return Ok(options);
    };
    let Some(dictionary) = arg.as_object() else {
        if !arg.to_boolean() {
            options.block = Position::End;
        }
        return Ok(options);
    };
    scroll_behavior(&dictionary, context)?;
    options.block = position(&dictionary, "block", Position::Start, context)?;
    let container = dictionary.get(JsString::from("container"), context)?;
    if !container.is_undefined() {
        match container
            .to_string(context)?
            .to_std_string_escaped()
            .as_str()
        {
            "all" => {}
            "nearest" => options.nearest_container = true,
            _ => {
                return Err(JsNativeError::typ()
                    .with_message("container is not a valid ScrollIntoViewContainer")
                    .into());
            }
        }
    }
    options.inline = position(&dictionary, "inline", Position::Nearest, context)?;
    Ok(options)
}

/// The new offset of one viewport axis that brings the element's span
/// `low..high` into the view `offset..offset + size`. `start_is_low` says
/// whether the axis starts at its low physical edge.
///
/// Spec: <https://drafts.csswg.org/cssom-view/#determine-the-scroll-into-view-position>
fn axis_offset(
    position: Position,
    start_is_low: bool,
    (low, high): (f64, f64),
    offset: f64,
    size: f64,
) -> f64 {
    let align_low = low;
    let align_high = high - size;
    match position {
        Position::Start if start_is_low => align_low,
        Position::Start => align_high,
        Position::End if start_is_low => align_high,
        Position::End => align_low,
        Position::Center => (low + high - size) / 2.0,
        Position::Nearest => {
            let extent = high - low;
            let low_out = low < offset;
            let high_out = high > offset + size;
            match (low_out, high_out) {
                (true, true) | (false, false) => offset,
                (true, false) if extent <= size => align_low,
                (true, false) => align_high,
                (false, true) if extent > size => align_low,
                (false, true) => align_high,
            }
        }
    }
}

/// Whether `index`, an ancestor of the target, is a scroll container: it
/// computes an `overflow` other than `visible` or `clip` on an axis.
fn is_scroll_container(context: &mut Context, index: usize) -> JsResult<bool> {
    for property in ["overflow-x", "overflow-y"] {
        let value = read_computed(context, index, property)?;
        if matches!(value.as_deref(), Some("hidden" | "scroll" | "auto")) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether a scroll container other than the viewport sits on the
/// containing-block chain between `index` and the document element
/// `root`: an absolutely positioned box skips the static ancestors
/// between it and its positioned containing block. The document element's
/// `overflow` applies to the viewport, so it does not count; neither does
/// the body's when the root's is `visible` on both axes, since the body's
/// then applies to the viewport instead (CSS Overflow 3 §3.3).
fn has_inner_scroll_container(context: &mut Context, index: usize, root: usize) -> JsResult<bool> {
    let (body, mut current) = with_state(context, |s| {
        let document = s.host.document();
        let body = first_element_child(document, root, Some("body"));
        (body, document.parent_of(index))
    })?;
    let mut root_visible = true;
    for property in ["overflow-x", "overflow-y"] {
        let value = read_computed(context, root, property)?;
        root_visible &= value.as_deref().is_none_or(|v| v == "visible");
    }
    let body = body.filter(|_| root_visible);
    let mut absolute =
        box_geometry(context, index)?.is_some_and(|g| g.position == PositionKind::Absolute);
    while let Some(ancestor) = current.filter(|&ancestor| ancestor != root) {
        // An ancestor without a box (`display: contents`, say) has no
        // scrolling box whatever its `overflow`, and contains nothing.
        let geometry = box_geometry(context, ancestor)?;
        let ancestor_position = geometry
            .as_ref()
            .map_or(PositionKind::Static, |g| g.position);
        if geometry.is_some() && !(absolute && ancestor_position == PositionKind::Static) {
            if Some(ancestor) != body && is_scroll_container(context, ancestor)? {
                return Ok(true);
            }
            absolute = ancestor_position == PositionKind::Absolute;
        }
        current = with_state(context, |s| s.host.document().parent_of(ancestor))?;
    }
    Ok(false)
}

/// Whether an ancestor of `index` is fixed-position, which keeps `index`
/// fixed with respect to the viewport too. An ancestor with no box does
/// not count.
fn has_fixed_ancestor(context: &mut Context, index: usize) -> JsResult<bool> {
    let mut current = with_state(context, |s| s.host.document().parent_of(index))?;
    while let Some(ancestor) = current {
        if box_geometry(context, ancestor)?.is_some_and(|g| g.position == PositionKind::Fixed) {
            return Ok(true);
        }
        current = with_state(context, |s| s.host.document().parent_of(ancestor))?;
    }
    Ok(false)
}

/// `Element.scrollIntoView(arg)`: scroll the viewport so the element's
/// border box is aligned as `block` and `inline` ask, along the axes of
/// the document element's writing mode. An element without a box, a
/// fixed-position one or one inside a fixed-position ancestor (the
/// viewport does not move either), or one that
/// `container: "nearest"` confines to an inner scroll container leaves the
/// viewport where it is. Every `behavior` scrolls instantly, so the
/// returned promise resolves to a non-interrupted `ScrollResult`.
pub(crate) fn scroll_into_view(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let promise = match scroll(this, args, context) {
        Ok(()) => {
            let result = ObjectInitializer::new(context)
                .property(js_string!("interrupted"), false, Attribute::all())
                .build();
            JsPromise::resolve(result, context)?
        }
        // An operation returning a promise reports a thrown exception
        // through it (WebIDL §3.2.6); an uncatchable one still aborts.
        Err(error) if error.as_engine().is_none() => JsPromise::reject(error, context)?,
        Err(error) => return Err(error),
    };
    Ok(promise.into())
}

/// The scroll behind [`scroll_into_view`], which wraps its outcome in the
/// returned promise.
fn scroll(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<()> {
    let index = this_element(this, context)?;
    let options = options(args.first(), context)?;
    let Some(geometry) = box_geometry(context, index)? else {
        return Ok(());
    };
    if geometry.position == PositionKind::Fixed || has_fixed_ancestor(context, index)? {
        return Ok(());
    }
    let root = with_state(context, |s| {
        let document = s.host.document();
        first_element_child(document, document.root_index(), None)
    })?;
    // cov:ignore: an element with a box is connected, so the document has a root.
    let Some(root) = root else {
        return Ok(());
    };
    if options.nearest_container && has_inner_scroll_container(context, index, root)? {
        return Ok(());
    }
    let writing_mode = read_computed(context, root, "writing-mode")?;
    let direction = read_computed(context, root, "direction")?;
    let ltr = direction.as_deref() != Some("rtl");
    // Which physical axis is the block axis, and where each axis starts.
    let (vertical, block_start_is_low, inline_start_is_low) = match writing_mode.as_deref() {
        Some("vertical-rl" | "sideways-rl") => (true, false, ltr),
        Some("vertical-lr") => (true, true, ltr),
        Some("sideways-lr") => (true, true, !ltr),
        _ => (false, true, ltr),
    };
    let viewport = with_state(context, |s| s.host.viewport_size())?;
    // A document without a viewport has nothing to scroll.
    let Some((width, height)) =
        viewport.map_err(|error| super::webidl::host_failure(context, error))?
    else {
        return Ok(());
    };
    let (scroll_x, scroll_y) = with_state(context, |s| (s.viewport_scroll_x, s.viewport_scroll_y))?;
    let border = geometry.border_box;
    let horizontal = (border.left, border.right);
    let vertical_span = (border.top, border.bottom);
    let (x_position, x_start_is_low, y_position, y_start_is_low) = if vertical {
        (
            options.block,
            block_start_is_low,
            options.inline,
            inline_start_is_low,
        )
    } else {
        (
            options.inline,
            inline_start_is_low,
            options.block,
            block_start_is_low,
        )
    };
    let x = axis_offset(x_position, x_start_is_low, horizontal, scroll_x, width);
    let y = axis_offset(y_position, y_start_is_low, vertical_span, scroll_y, height);
    with_state(context, |s| {
        s.viewport_scroll_x = clamp_viewport_offset(x);
        s.viewport_scroll_y = clamp_viewport_offset(y);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests;
