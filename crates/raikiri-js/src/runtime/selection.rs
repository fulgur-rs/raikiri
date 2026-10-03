//! The bounded DOM Range and Selection API used by script-driven rendering.

use std::cell::Cell;

use boa_engine::object::JsObject;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsResult, JsValue, Trace, js_string};
use raikiri_dom::NodeKind;

use super::interfaces::{Members, function, protos, wrap};
use super::webidl::{arg_node, arg_unsigned_long, this_document, throw_dom_exception, with_state};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoundaryPoints {
    start_container: usize,
    start_offset: usize,
    end_container: usize,
    end_offset: usize,
}

impl BoundaryPoints {
    fn collapsed(self) -> bool {
        self.start_container == self.end_container && self.start_offset == self.end_offset
    }
}

/// Native boundary-point state for a script-visible Range.
#[derive(Debug, Trace, Finalize, JsData)]
struct RangeData {
    #[unsafe_ignore_trace]
    points: Cell<BoundaryPoints>,
}

/// Native brand for the singleton Selection interface object.
#[derive(Debug, Trace, Finalize, JsData)]
struct SelectionData;

fn range_points(value: &JsValue) -> JsResult<BoundaryPoints> {
    value
        .as_object()
        .and_then(|object| {
            object
                .downcast_ref::<RangeData>()
                .map(|data| data.points.get())
        })
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("'this' is not a Range")
                .into()
        })
}

fn is_selection(value: &JsValue) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.downcast_ref::<SelectionData>().is_some())
}

fn selection_ranges(this: &JsValue, context: &mut Context) -> JsResult<Vec<JsObject>> {
    if !is_selection(this) {
        return Err(JsNativeError::typ()
            .with_message("'this' is not a Selection")
            .into());
    }
    with_state(context, |state| state.selection_ranges.clone())
}

fn selected_range(this: &JsValue, context: &mut Context) -> JsResult<Option<JsObject>> {
    Ok(selection_ranges(this, context)?.into_iter().next())
}

fn range_start_container(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let index = range_points(this)?.start_container;
    Ok(wrap(context, index)?.into())
}

fn range_end_container(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = range_points(this)?.end_container;
    Ok(wrap(context, index)?.into())
}

fn range_start_offset(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(range_points(this)?.start_offset as u32))
}

fn range_end_offset(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(range_points(this)?.end_offset as u32))
}

fn range_collapsed(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(range_points(this)?.collapsed()))
}

fn node_length(context: &mut Context, index: usize) -> JsResult<usize> {
    let length = with_state(context, |state| {
        let document = state.host.document();
        let node = document.get_node(index)?;
        Some(match node.kind() {
            NodeKind::Document | NodeKind::Element | NodeKind::DocumentFragment => {
                node.children.len()
            }
            NodeKind::Text | NodeKind::Comment | NodeKind::ProcessingInstruction => document
                .character_data(index)
                .unwrap_or_default()
                .encode_utf16()
                .count(),
            // `NodeKind` is non-exhaustive; unknown future node kinds have no
            // child or character data exposed by this runtime.
            _ => 0, // cov:ignore: no NodeKind variant beyond this exhaustive set exists in the current dependency build.
        })
    })?;
    length.ok_or_else(|| {
        JsNativeError::typ()
            .with_message("node handle is out of range")
            .into()
    })
}

fn range_select_node_contents(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    range_points(this)?;
    let node = arg_node(args, 0, context)?;
    let length = node_length(context, node)?;
    let points = BoundaryPoints {
        start_container: node,
        start_offset: 0,
        end_container: node,
        end_offset: length,
    };
    let updated = this.as_object().and_then(|object| {
        object
            .downcast_ref::<RangeData>()
            .map(|data| data.points.set(points))
    });
    // The Range brand was checked above, and no script callback runs before this update.
    // cov:ignore: this defensive guard cannot fail between the immediately preceding brand check and update.
    if updated.is_none() {
        return Err(JsNativeError::typ()
            .with_message("'this' is not a Range")
            .into());
    }
    Ok(JsValue::undefined())
}

pub(crate) const RANGE_MEMBERS: Members = Members {
    getters: &[
        ("startContainer", range_start_container),
        ("startOffset", range_start_offset),
        ("endContainer", range_end_container),
        ("endOffset", range_end_offset),
        ("collapsed", range_collapsed),
    ],
    accessors: &[],
    methods: &[("selectNodeContents", 1, range_select_node_contents)],
};

pub(crate) fn create_range(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let document = this_document(this, context)?;
    let points = BoundaryPoints {
        start_container: document,
        start_offset: 0,
        end_container: document,
        end_offset: 0,
    };
    let object = JsObject::from_proto_and_data(
        Some(protos(context).range.clone()),
        RangeData {
            points: Cell::new(points),
        },
    );
    Ok(object.into())
}

fn selection_range_count(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::from(selection_ranges(this, context)?.len() as u32))
}

fn selection_is_collapsed(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(range) = selected_range(this, context)? else {
        return Ok(JsValue::from(true));
    };
    Ok(JsValue::from(range_points_object(&range)?.collapsed()))
}

fn selection_boundary(this: &JsValue, context: &mut Context, start: bool) -> JsResult<JsValue> {
    let Some(range) = selected_range(this, context)? else {
        return Ok(JsValue::null());
    };
    let points = range_points_object(&range)?;
    let index = if start {
        points.start_container
    } else {
        points.end_container
    };
    Ok(wrap(context, index)?.into())
}

fn selection_anchor_node(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    selection_boundary(this, context, true)
}

fn selection_focus_node(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    selection_boundary(this, context, false)
}

fn selection_boundary_offset(
    this: &JsValue,
    context: &mut Context,
    start: bool,
) -> JsResult<JsValue> {
    let Some(range) = selected_range(this, context)? else {
        return Ok(JsValue::from(0));
    };
    let points = range_points_object(&range)?;
    Ok(JsValue::from(if start {
        points.start_offset
    } else {
        points.end_offset
    } as u32))
}

fn selection_anchor_offset(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    selection_boundary_offset(this, context, true)
}

fn selection_focus_offset(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    selection_boundary_offset(this, context, false)
}

fn selection_add_range(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    selection_ranges(this, context)?;
    let range = args
        .first()
        .and_then(JsValue::as_object)
        .filter(|object| object.downcast_ref::<RangeData>().is_some())
        .ok_or_else(|| JsNativeError::typ().with_message("argument is not a Range"))?;
    with_state(context, |state| {
        if state.selection_ranges.is_empty() {
            state.selection_ranges.push(range);
        }
    })?;
    Ok(JsValue::undefined())
}

fn selection_get_range_at(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let ranges = selection_ranges(this, context)?;
    let index = arg_unsigned_long(args, 0, context)?;
    let Some(range) = ranges.get(index) else {
        return Err(throw_dom_exception(
            context,
            "IndexSizeError",
            "range index is outside the selection",
        ));
    };
    Ok(range.clone().into())
}

fn selection_remove_all_ranges(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    selection_ranges(this, context)?;
    with_state(context, |state| state.selection_ranges.clear())?;
    Ok(JsValue::undefined())
}

pub(crate) const SELECTION_MEMBERS: Members = Members {
    getters: &[
        ("anchorNode", selection_anchor_node),
        ("anchorOffset", selection_anchor_offset),
        ("focusNode", selection_focus_node),
        ("focusOffset", selection_focus_offset),
        ("isCollapsed", selection_is_collapsed),
        ("rangeCount", selection_range_count),
    ],
    accessors: &[],
    methods: &[
        ("addRange", 1, selection_add_range),
        ("getRangeAt", 1, selection_get_range_at),
        ("removeAllRanges", 0, selection_remove_all_ranges),
    ],
};

fn range_points_object(object: &JsObject) -> JsResult<BoundaryPoints> {
    object
        .downcast_ref::<RangeData>()
        .map(|data| data.points.get())
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("value is not a Range")
                .into()
        })
}

pub(crate) fn window_get_selection(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let existing = with_state(context, |state| state.selection_object.clone())?;
    if let Some(existing) = existing {
        return Ok(existing.into());
    }
    let selection =
        JsObject::from_proto_and_data(Some(protos(context).selection.clone()), SelectionData);
    with_state(context, |state| {
        state.selection_object = Some(selection.clone());
    })?;
    Ok(selection.into())
}

pub(crate) fn install_window(context: &mut Context) -> JsResult<()> {
    let get_selection = function(context, "getSelection", 0, window_get_selection)?;
    let descriptor = PropertyDescriptor::builder()
        .value(get_selection)
        .writable(true)
        .enumerable(true)
        .configurable(true)
        .build();
    let global = context.global_object();
    global.define_property_or_throw(js_string!("getSelection"), descriptor, context)?;
    Ok(())
}

#[cfg(test)]
mod tests;
