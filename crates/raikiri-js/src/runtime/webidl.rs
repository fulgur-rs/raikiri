//! WebIDL-level conversions and checks shared by every binding.

use boa_engine::{Context, JsError, JsNativeError, JsResult, JsValue};
use raikiri_dom::NodeKind;

use super::host::HostError;
use super::interfaces::{DomExceptionData, NodeHandle, protos};
use super::{State, shared};

/// `DOMString` conversion of argument `i` (missing -> "undefined", per ToString).
pub(crate) fn dom_string(args: &[JsValue], i: usize, context: &mut Context) -> JsResult<String> {
    Ok(args
        .get(i)
        .cloned()
        .unwrap_or_default()
        .to_string(context)?
        .to_std_string_escaped())
}

/// WebIDL `unsigned long` conversion (ToUint32, no `[EnforceRange]`) of
/// required argument `i`; a missing argument is a `TypeError`.
pub(crate) fn arg_unsigned_long(
    args: &[JsValue],
    i: usize,
    context: &mut Context,
) -> JsResult<usize> {
    let value = args
        .get(i)
        .ok_or_else(|| type_error("a required argument is missing"))?;
    Ok(value.to_u32(context)? as usize)
}

/// The arena index behind a node wrapper, if `value` is one. Not an error
/// by itself -- callers that accept a mix of `Node` and non-`Node`
/// arguments (DOM §4.2.6 "convert nodes into a node") use this to tell them
/// apart before deciding how to convert each one.
pub(crate) fn node_index(value: &JsValue) -> Option<usize> {
    value
        .as_object()?
        .downcast_ref::<NodeHandle>()
        .map(|h| h.index)
}

fn type_error(message: &str) -> JsError {
    JsNativeError::typ().with_message(message.to_owned()).into()
}

fn kind_of(context: &mut Context, index: usize) -> JsResult<NodeKind> {
    with_state(context, |state| {
        state.host.document().get_node(index).map(|n| n.kind())
    })?
    .ok_or_else(|| type_error("node handle is out of range"))
}

/// Brand check for `Node` members.
pub(crate) fn this_node(this: &JsValue, _context: &mut Context) -> JsResult<usize> {
    node_index(this).ok_or_else(|| type_error("'this' is not a Node"))
}

/// Brand check for `Element` members.
pub(crate) fn this_element(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_node(this, context)?;
    match kind_of(context, index)? {
        NodeKind::Element => Ok(index),
        _ => Err(type_error("'this' is not an Element")),
    }
}

/// Brand check for `Document` members.
pub(crate) fn this_document(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_node(this, context)?;
    match kind_of(context, index)? {
        NodeKind::Document => Ok(index),
        _ => Err(type_error("'this' is not a Document")),
    }
}

/// Brand check for the `ParentNode` mixin (Document, DocumentFragment,
/// Element).
pub(crate) fn this_parent_node(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_node(this, context)?;
    match kind_of(context, index)? {
        NodeKind::Document | NodeKind::DocumentFragment | NodeKind::Element => Ok(index),
        _ => Err(type_error("'this' does not implement ParentNode")),
    }
}

/// Brand check for the `NonElementParentNode` mixin (Document,
/// DocumentFragment; notably not Element).
pub(crate) fn this_non_element_parent_node(
    this: &JsValue,
    context: &mut Context,
) -> JsResult<usize> {
    let index = this_node(this, context)?;
    match kind_of(context, index)? {
        NodeKind::Document | NodeKind::DocumentFragment => Ok(index),
        _ => Err(type_error("'this' does not implement NonElementParentNode")),
    }
}

/// Brand check for the `ChildNode` / `NonDocumentTypeChildNode` mixins
/// (Element, and every `CharacterData` interface: Text, Comment,
/// ProcessingInstruction).
pub(crate) fn this_child_node(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_node(this, context)?;
    match kind_of(context, index)? {
        NodeKind::Element
        | NodeKind::Text
        | NodeKind::Comment
        | NodeKind::ProcessingInstruction => Ok(index),
        _ => Err(type_error("'this' does not implement ChildNode")),
    }
}

/// Brand check for `CharacterData` members (Text, Comment,
/// ProcessingInstruction).
pub(crate) fn this_character_data(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_node(this, context)?;
    match kind_of(context, index)? {
        NodeKind::Text | NodeKind::Comment | NodeKind::ProcessingInstruction => Ok(index),
        _ => Err(type_error("'this' is not a CharacterData node")),
    }
}

/// Brand check for `ProcessingInstruction` members.
pub(crate) fn this_processing_instruction(
    this: &JsValue,
    context: &mut Context,
) -> JsResult<usize> {
    let index = this_node(this, context)?;
    match kind_of(context, index)? {
        NodeKind::ProcessingInstruction => Ok(index),
        _ => Err(type_error("'this' is not a ProcessingInstruction")),
    }
}

/// A `Node` argument (WebIDL interface-type conversion).
pub(crate) fn arg_node(args: &[JsValue], i: usize, _context: &mut Context) -> JsResult<usize> {
    args.get(i)
        .and_then(node_index)
        .ok_or_else(|| type_error("argument is not a Node"))
}

/// A nullable `Node` argument (`Node?`) that is itself *optional* (has an
/// implied `null` default, e.g. `Node.contains`'s `other`): a missing
/// argument, `null`, and `undefined` all convert to `None`; anything else
/// must be a `Node` wrapper or this throws `TypeError`, the same as
/// [`arg_node`].
pub(crate) fn arg_node_or_null(
    args: &[JsValue],
    i: usize,
    _context: &mut Context,
) -> JsResult<Option<usize>> {
    match args.get(i) {
        None => Ok(None),
        Some(v) if v.is_null() || v.is_undefined() => Ok(None),
        Some(v) => node_index(v)
            .map(Some)
            .ok_or_else(|| type_error("argument is not a Node")),
    }
}

/// A nullable `Node` argument (`Node?`) that is *required* (no default,
/// e.g. `Node.insertBefore`'s `child`): WebIDL's operation-arity check for a
/// non-optional parameter runs before any per-argument conversion, so a call
/// that omits it outright is a `TypeError` regardless of the parameter's own
/// nullable type -- distinct from an explicit `null`/`undefined` in that
/// same position, which still converts to `None`, the same as
/// [`arg_node_or_null`].
pub(crate) fn arg_required_node_or_null(
    args: &[JsValue],
    i: usize,
    context: &mut Context,
) -> JsResult<Option<usize>> {
    if args.len() <= i {
        return Err(type_error("a required argument is missing"));
    }
    arg_node_or_null(args, i, context)
}

/// A fixed-message `Error` for a raikiri-dom mutation `Result<_, String>`
/// failure that a preceding brand/kind check has already made unreachable
/// in practice (every call site is `cov:ignore`d for exactly that reason).
/// Takes no argument on purpose: raikiri-dom's own message text can embed
/// an arena index (e.g. "target index 3 is out of range"), which must
/// never reach script, and there is no reachable path here to describe
/// more specifically anyway, so the caller's `String` is simply dropped
/// rather than threaded through and ignored.
pub(crate) fn unreachable_mutation_error() -> JsError {
    JsNativeError::error()
        .with_message("an internal DOM operation failed unexpectedly")
        .into()
}

/// Create a `DOMException` with the given name and return it as a JS error.
pub(crate) fn throw_dom_exception(context: &mut Context, name: &str, message: &str) -> JsError {
    let proto = protos(context).dom_exception.clone();
    let object = boa_engine::JsObject::from_proto_and_data(
        Some(proto),
        DomExceptionData {
            name: name.to_owned(),
            message: message.to_owned(),
        },
    );
    JsError::from_opaque(object.into())
}

/// Record a host failure for the harness, then surface it as an `Error`.
pub(crate) fn host_failure(context: &mut Context, error: HostError) -> JsError {
    let visible_message = error.0.clone();
    host_failure_with_message(context, error, visible_message)
}

/// Record a host failure for the harness (the same as [`host_failure`]), but
/// surface a caller-supplied message to script instead of the failure's own
/// text. Some host failure messages carry an internal detail -- such as a
/// raikiri-dom arena node index -- that must never be observable from
/// script; the harness-facing recorded failure keeps the original message.
pub(crate) fn host_failure_with_message(
    context: &mut Context,
    error: HostError,
    visible_message: impl Into<String>,
) -> JsError {
    record_failure(context, &error.0);
    JsNativeError::error()
        .with_message(visible_message.into())
        .into()
}

/// Remember `message` as the evaluation's host failure unless one is
/// already recorded. When the state is borrowed elsewhere, the message is
/// parked in the context and merged by [`super::DomRuntime::evaluate`].
fn record_failure(context: &mut Context, message: &str) {
    let shared = shared(context);
    // A parked failure predates anything recorded now, so it keeps priority.
    let parked = context.get_data::<DeferredHostFailure>().is_some();
    if let Ok(mut state) = shared.0.try_borrow_mut() {
        if !parked {
            state.host_failure.get_or_insert_with(|| message.to_owned());
        }
    } else if !parked {
        context.insert_data(DeferredHostFailure(message.to_owned()));
    }
}

/// A host failure that could not be written into [`State`] because the
/// state was already borrowed.
pub(crate) struct DeferredHostFailure(pub String);

/// Run `f` with the shared state borrowed. A re-entrant borrow (a binding
/// called while another holds the state) is recorded as a host failure and
/// thrown as an `Error` rather than panicking.
pub(crate) fn with_state<T>(context: &mut Context, f: impl FnOnce(&mut State) -> T) -> JsResult<T> {
    let shared = shared(context);
    let Ok(mut state) = shared.0.try_borrow_mut() else {
        const MESSAGE: &str = "re-entrant DOM runtime access";
        record_failure(context, MESSAGE);
        return Err(JsNativeError::error().with_message(MESSAGE).into());
    };
    Ok(f(&mut state))
}
