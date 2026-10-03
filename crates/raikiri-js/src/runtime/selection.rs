//! The bounded DOM Range and Selection API used by script-driven rendering.

use std::cell::Cell;

use boa_engine::gc::GcRefCell;
use boa_engine::object::JsObject;
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsResult, JsValue, Trace, js_string};
use raikiri_dom::NodeKind;

use super::interfaces::{Members, function, protos, wrap};
use super::webidl::{
    arg_node, arg_unsigned_long, dom_string, this_document, throw_dom_exception, with_state,
};

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

#[derive(Debug, Trace, Finalize, JsData)]
struct TreeWalkerData {
    #[unsafe_ignore_trace]
    root: usize,
    #[unsafe_ignore_trace]
    current_node: Cell<usize>,
    #[unsafe_ignore_trace]
    what_to_show: u32,
}

#[derive(Debug, Trace, Finalize, JsData)]
struct HighlightData {
    ranges: GcRefCell<Vec<JsObject>>,
}

#[derive(Debug, Trace, Finalize, JsData)]
struct HighlightRegistryData;

/// A custom highlight range recorded by the script runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomHighlightRange {
    /// The name used to register this highlight in `CSS.highlights`.
    pub name: String,
    /// The arena index of the range's start node.
    pub start_container: usize,
    /// UTF-16 offset into the start node.
    pub start_offset: usize,
    /// The arena index of the range's end node.
    pub end_container: usize,
    /// UTF-16 offset into the end node.
    pub end_offset: usize,
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

fn range_set_boundary(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
    start: bool,
) -> JsResult<JsValue> {
    let mut points = range_points(this)?;
    let container = arg_node(args, 0, context)?;
    let offset = arg_unsigned_long(args, 1, context)?;
    if offset > node_length(context, container)? {
        return Err(throw_dom_exception(
            context,
            "IndexSizeError",
            "range offset is outside the node",
        ));
    }
    if start {
        points.start_container = container;
        points.start_offset = offset;
    } else {
        points.end_container = container;
        points.end_offset = offset;
    }
    let updated = this.as_object().and_then(|object| {
        object
            .downcast_ref::<RangeData>()
            .map(|data| data.points.set(points))
    });
    if updated.is_none() {
        return Err(JsNativeError::typ()
            .with_message("'this' is not a Range")
            .into());
    }
    Ok(JsValue::undefined())
}

fn range_set_start(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    range_set_boundary(this, args, context, true)
}

fn range_set_end(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    range_set_boundary(this, args, context, false)
}

fn tree_walker_object(this: &JsValue) -> JsResult<JsObject> {
    let object = this
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("'this' is not a TreeWalker"))?;
    if object.downcast_ref::<TreeWalkerData>().is_none() {
        return Err(JsNativeError::typ()
            .with_message("'this' is not a TreeWalker")
            .into());
    }
    Ok(object)
}

fn tree_walker_root(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let object = tree_walker_object(this)?;
    let root = object
        .downcast_ref::<TreeWalkerData>()
        .expect("TreeWalker brand was checked")
        .root;
    Ok(wrap(context, root)?.into())
}

fn tree_walker_current_node(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let object = tree_walker_object(this)?;
    let current_node = object
        .downcast_ref::<TreeWalkerData>()
        .expect("TreeWalker brand was checked")
        .current_node
        .get();
    Ok(wrap(context, current_node)?.into())
}

fn node_is_within_root(document: &raikiri_dom::Document, node: usize, root: usize) -> bool {
    let mut current = Some(node);
    while let Some(index) = current {
        if index == root {
            return true;
        }
        current = document.parent_of(index);
    }
    false
}

fn tree_walker_set_current_node(
    this: &JsValue,
    value: &JsValue,
    context: &mut Context,
) -> JsResult<()> {
    let object = tree_walker_object(this)?;
    let root = object
        .downcast_ref::<TreeWalkerData>()
        .expect("TreeWalker brand was checked")
        .root;
    let index = arg_node(std::slice::from_ref(value), 0, context)?;
    let inside = with_state(context, |state| {
        node_is_within_root(state.host.document(), index, root)
    })?;
    if !inside {
        return Err(throw_dom_exception(
            context,
            "NotSupportedError",
            "currentNode is outside the TreeWalker root",
        ));
    }
    object
        .downcast_ref::<TreeWalkerData>()
        .expect("TreeWalker brand was checked")
        .current_node
        .set(index);
    Ok(())
}

fn tree_walker_current_node_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let value = args
        .first()
        .ok_or_else(|| JsNativeError::typ().with_message("a required argument is missing"))?;
    tree_walker_set_current_node(this, value, context)?;
    Ok(JsValue::undefined())
}

fn tree_walker_what_to_show(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let object = tree_walker_object(this)?;
    let what_to_show = object
        .downcast_ref::<TreeWalkerData>()
        .expect("TreeWalker brand was checked")
        .what_to_show;
    Ok(JsValue::from(what_to_show))
}

fn next_tree_node(document: &raikiri_dom::Document, root: usize, current: usize) -> Option<usize> {
    if let Some(child) = document.get_node(current)?.children.first() {
        return Some(*child);
    }
    let mut index = current;
    while index != root {
        let parent = document.parent_of(index)?;
        let siblings = &document.get_node(parent)?.children;
        let position = siblings.iter().position(|sibling| *sibling == index)?;
        if let Some(next) = siblings.get(position + 1) {
            return Some(*next);
        }
        index = parent;
    }
    None
}

fn tree_walker_next_node(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let object = tree_walker_object(this)?;
    let (root, what_to_show, current) = {
        let data = object
            .downcast_ref::<TreeWalkerData>()
            .expect("TreeWalker brand was checked");
        (data.root, data.what_to_show, data.current_node.get())
    };
    let next = with_state(context, |state| {
        let document = state.host.document();
        let mut candidate = next_tree_node(document, root, current);
        while let Some(index) = candidate {
            let kind = document.get_node(index).map(|node| node.kind());
            let accepted = match kind {
                Some(NodeKind::Text) => what_to_show & (1 << 2) != 0,
                _ => false,
            };
            if accepted {
                return Some(index);
            }
            candidate = next_tree_node(document, root, index);
        }
        None
    })?;
    let Some(next) = next else {
        return Ok(JsValue::null());
    };
    object
        .downcast_ref::<TreeWalkerData>()
        .expect("TreeWalker brand was checked")
        .current_node
        .set(next);
    Ok(wrap(context, next)?.into())
}

pub(crate) const TREE_WALKER_MEMBERS: Members = Members {
    getters: &[
        ("root", tree_walker_root),
        ("whatToShow", tree_walker_what_to_show),
    ],
    accessors: &[(
        "currentNode",
        tree_walker_current_node,
        tree_walker_current_node_set,
    )],
    methods: &[("nextNode", 0, tree_walker_next_node)],
};

pub(crate) fn create_tree_walker(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    this_document(this, context)?;
    let root = arg_node(args, 0, context)?;
    let what_to_show = match args.get(1) {
        Some(_) => arg_unsigned_long(args, 1, context)? as u32,
        None => u32::MAX,
    };
    Ok(JsObject::from_proto_and_data(
        Some(protos(context).tree_walker.clone()),
        TreeWalkerData {
            root,
            current_node: Cell::new(root),
            what_to_show,
        },
    )
    .into())
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
    methods: &[
        ("selectNodeContents", 1, range_select_node_contents),
        ("setStart", 2, range_set_start),
        ("setEnd", 2, range_set_end),
    ],
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

fn highlight_add(this: &JsValue, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let highlight_object = this
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("'this' is not a Highlight"))?;
    let highlight = highlight_object
        .downcast_ref::<HighlightData>()
        .ok_or_else(|| JsNativeError::typ().with_message("'this' is not a Highlight"))?;
    let range = args
        .first()
        .and_then(JsValue::as_object)
        .filter(|object| object.downcast_ref::<RangeData>().is_some())
        .ok_or_else(|| JsNativeError::typ().with_message("argument is not a Range"))?;
    highlight.ranges.borrow_mut().push(range);
    Ok(this.clone())
}

pub(crate) const HIGHLIGHT_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[],
    methods: &[("add", 1, highlight_add)],
};

pub(crate) fn highlight_constructor(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if this.is_undefined() {
        return Err(JsNativeError::typ()
            .with_message("Constructor Highlight requires 'new'")
            .into());
    }
    Ok(JsObject::from_proto_and_data(
        Some(protos(context).highlight.clone()),
        HighlightData {
            ranges: GcRefCell::new(Vec::new()),
        },
    )
    .into())
}

fn is_highlight_registry(this: &JsValue) -> bool {
    this.as_object()
        .is_some_and(|object| object.downcast_ref::<HighlightRegistryData>().is_some())
}

fn highlight_registry_get(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if !is_highlight_registry(this) {
        return Err(JsNativeError::typ()
            .with_message("'this' is not a HighlightRegistry")
            .into());
    }
    let name = dom_string(args, 0, context)?;
    Ok(
        with_state(context, |state| state.custom_highlights.get(&name).cloned())?
            .map_or_else(JsValue::undefined, JsValue::from),
    )
}

fn highlight_registry_set(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if !is_highlight_registry(this) {
        return Err(JsNativeError::typ()
            .with_message("'this' is not a HighlightRegistry")
            .into());
    }
    let name = dom_string(args, 0, context)?;
    let highlight = args
        .get(1)
        .and_then(JsValue::as_object)
        .filter(|object| object.downcast_ref::<HighlightData>().is_some())
        .ok_or_else(|| JsNativeError::typ().with_message("value is not a Highlight"))?;
    with_state(context, |state| {
        state.custom_highlights.insert(name, highlight.clone());
    })?;
    Ok(this.clone())
}

pub(crate) fn install_globals(context: &mut Context) -> JsResult<()> {
    let node_filter = JsObject::from_proto_and_data(
        Some(context.intrinsics().constructors().object().prototype()),
        (),
    );
    let show_text = PropertyDescriptor::builder()
        .value(1 << 2)
        .writable(false)
        .enumerable(true)
        .configurable(false)
        .build();
    node_filter.define_property_or_throw(js_string!("SHOW_TEXT"), show_text, context)?;
    context.register_global_property(
        js_string!("NodeFilter"),
        node_filter,
        Attribute::CONFIGURABLE,
    )?;

    let registry = JsObject::from_proto_and_data(
        Some(context.intrinsics().constructors().object().prototype()),
        HighlightRegistryData,
    );
    for (name, length, native) in [
        (
            "get",
            1,
            highlight_registry_get as boa_engine::native_function::NativeFunctionPointer,
        ),
        (
            "set",
            2,
            highlight_registry_set as boa_engine::native_function::NativeFunctionPointer,
        ),
    ] {
        let operation = function(context, name, length, native)?;
        let descriptor = PropertyDescriptor::builder()
            .value(operation)
            .writable(true)
            .enumerable(true)
            .configurable(true)
            .build();
        registry.define_property_or_throw(js_string!(name), descriptor, context)?;
    }
    let css = context
        .global_object()
        .get(js_string!("CSS"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("CSS namespace is missing"))?;
    let highlights = PropertyDescriptor::builder()
        .value(registry)
        .writable(false)
        .enumerable(true)
        .configurable(true)
        .build();
    css.define_property_or_throw(js_string!("highlights"), highlights, context)?;
    Ok(())
}

/// Read the current highlight ranges while their node indices still refer to the live document.
pub(crate) fn custom_highlight_ranges(
    context: &mut Context,
) -> JsResult<Vec<CustomHighlightRange>> {
    let entries = with_state(context, |state| {
        state
            .custom_highlights
            .iter()
            .map(|(name, highlight)| (name.clone(), highlight.clone()))
            .collect::<Vec<_>>()
    })?;
    let mut output = Vec::new();
    for (name, highlight) in entries {
        let Some(data) = highlight.downcast_ref::<HighlightData>() else {
            return Err(JsNativeError::typ()
                .with_message("registered value is not a Highlight")
                .into());
        };
        for range in data.ranges.borrow().iter() {
            let points = range_points_object(range)?;
            output.push(CustomHighlightRange {
                name: name.clone(),
                start_container: points.start_container,
                start_offset: points.start_offset,
                end_container: points.end_container,
                end_offset: points.end_offset,
            });
        }
    }
    Ok(output)
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
