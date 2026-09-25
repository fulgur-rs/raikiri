//! WebIDL-level conversions and checks shared by every binding.

use boa_engine::{Context, JsError, JsNativeError, JsResult, JsValue};
use raikiri_dom::NodeKind;

use super::host::HostError;
use super::interfaces::{DomExceptionData, NodeHandle, protos};
use super::{State, shared};

/// `DOMString` conversion of argument `i` (missing -> "undefined", per ToString).
#[allow(dead_code, reason = "used by string-taking DOM members")]
pub(crate) fn dom_string(args: &[JsValue], i: usize, context: &mut Context) -> JsResult<String> {
    Ok(args
        .get(i)
        .cloned()
        .unwrap_or_default()
        .to_string(context)?
        .to_std_string_escaped())
}

/// The arena index behind a node wrapper, if `value` is one.
fn brand(value: &JsValue) -> Option<usize> {
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
    brand(this).ok_or_else(|| type_error("'this' is not a Node"))
}

/// Brand check for `Element` members.
#[allow(dead_code, reason = "used by Element members")]
pub(crate) fn this_element(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_node(this, context)?;
    match kind_of(context, index)? {
        NodeKind::Element => Ok(index),
        _ => Err(type_error("'this' is not an Element")),
    }
}

/// Brand check for `Document` members.
#[allow(dead_code, reason = "used by Document members")]
pub(crate) fn this_document(this: &JsValue, context: &mut Context) -> JsResult<usize> {
    let index = this_node(this, context)?;
    match kind_of(context, index)? {
        NodeKind::Document => Ok(index),
        _ => Err(type_error("'this' is not a Document")),
    }
}

/// A `Node` argument (WebIDL interface-type conversion).
#[allow(dead_code, reason = "used by node-taking DOM members")]
pub(crate) fn arg_node(args: &[JsValue], i: usize, _context: &mut Context) -> JsResult<usize> {
    args.get(i)
        .and_then(brand)
        .ok_or_else(|| type_error("argument is not a Node"))
}

/// Create a `DOMException` with the given name and return it as a JS error.
#[allow(dead_code, reason = "used by DOM members that throw DOMException")]
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
#[allow(dead_code, reason = "used by bindings that call into the host")]
pub(crate) fn host_failure(context: &mut Context, error: HostError) -> JsError {
    record_failure(context, &error.0);
    JsNativeError::error().with_message(error.0).into()
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
