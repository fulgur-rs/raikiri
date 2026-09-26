//! `EventTarget` listener registration and removal (DOM §2.7). The listener
//! state kept here, in [`super::State`], is consumed by [`super::dispatch`],
//! which implements `dispatchEvent` (DOM §2.9) over it.
//!
//! Spec ref: <https://dom.spec.whatwg.org/#interface-eventtarget>

use boa_engine::object::JsObject;
use boa_engine::property::Attribute;
use boa_engine::{Context, JsNativeError, JsResult, JsValue, js_string};

use super::interfaces::{Members, function};
use super::webidl::{dom_string, node_index, with_state};

/// One registered listener (DOM §2.7's "an event listener" struct, minus
/// the touch-target-list/removed fields no algorithm here needs).
#[derive(Clone)]
pub(crate) struct Listener {
    pub kind: String,
    pub callback: JsObject,
    pub capture: bool,
    pub once: bool,
    pub passive: bool,
    /// Whether this entry is the single slot an event handler IDL attribute
    /// (`onclick`, `onerror`, ...) manages, rather than an ordinary
    /// `addEventListener` registration -- see
    /// [`super::dispatch::install_window_handlers`] and the matching
    /// `HTMLElement` accessors.
    pub is_handler: bool,
}

/// `this`'s listener-storage key: `Some(index)` for a `Node` wrapper, or
/// `None` for the window/global object -- the only two kinds of object this
/// runtime exposes as an `EventTarget`. A native function receives `this`
/// unmodified (unlike an ECMAScript function's own sloppy-mode "substitute
/// the global object" behavior), so an unqualified `addEventListener(...)`
/// call reaches here with `this` still `undefined`; treating `undefined`
/// (and `null`, reachable the same way through `.call(null, ...)`) as the
/// window key is what makes that call, and `window.addEventListener(...)`
/// itself, both work.
pub(crate) fn this_event_target(this: &JsValue, context: &mut Context) -> JsResult<Option<usize>> {
    if let Some(index) = node_index(this) {
        return Ok(Some(index));
    }
    if this.is_undefined() || this.is_null() {
        return Ok(None);
    }
    let global = context.global_object();
    if this
        .as_object()
        .is_some_and(|o| JsObject::equals(&o, &global))
    {
        return Ok(None);
    }
    Err(JsNativeError::typ()
        .with_message("'this' does not implement EventTarget")
        .into())
}

/// `callback` (WebIDL nullable callback interface `EventListener?`):
/// `null`/`undefined`/an absent argument all convert to `None`; any other
/// non-object value cannot convert to a callback interface type and is a
/// `TypeError`.
fn convert_callback(args: &[JsValue], _context: &mut Context) -> JsResult<Option<JsObject>> {
    match args.get(1) {
        None => Ok(None),
        Some(v) if v.is_null() || v.is_undefined() => Ok(None),
        Some(v) => v.as_object().map(Some).ok_or_else(|| {
            JsNativeError::typ()
                .with_message("callback is not an object")
                .into()
        }),
    }
}

/// One `AddEventListenerOptions` field (`ToBoolean` of whatever is there;
/// absent or `undefined` is `false`, matching the dictionary member's own
/// default).
pub(crate) fn dict_flag(context: &mut Context, options: &JsObject, name: &str) -> JsResult<bool> {
    let value = options.get(js_string!(name), context)?;
    Ok(!value.is_undefined() && value.to_boolean())
}

/// `options`' `(capture, once, passive)`, from the WebIDL union
/// `(AddEventListenerOptions or boolean)` (default `{}`) `addEventListener`
/// uses: a boolean --or any other primitive, including `null`-- shorthand
/// sets only `capture` (`ToBoolean`); an object is read as the
/// dictionary's three fields.
fn parse_add_options(context: &mut Context, args: &[JsValue]) -> JsResult<(bool, bool, bool)> {
    match args.get(2) {
        None => Ok((false, false, false)),
        Some(v) if v.is_undefined() || v.is_null() => Ok((false, false, false)),
        Some(v) => match v.as_object() {
            Some(options) => Ok((
                dict_flag(context, &options, "capture")?,
                dict_flag(context, &options, "once")?,
                dict_flag(context, &options, "passive")?,
            )),
            None => Ok((v.to_boolean(), false, false)),
        },
    }
}

/// `options`' `capture`, from the WebIDL union `(EventListenerOptions or
/// boolean)` (default `{}`) `removeEventListener` uses -- unlike
/// `addEventListener`'s `AddEventListenerOptions`, `EventListenerOptions`
/// has no `once`/`passive` members, so an object passed here never has
/// those properties read (a `once` accessor with a throwing getter is
/// simply never invoked by `removeEventListener`, only by
/// `addEventListener`).
fn capture_from_remove_options(context: &mut Context, args: &[JsValue]) -> JsResult<bool> {
    match args.get(2) {
        None => Ok(false),
        Some(v) if v.is_undefined() || v.is_null() => Ok(false),
        Some(v) => match v.as_object() {
            Some(options) => dict_flag(context, &options, "capture"),
            None => Ok(v.to_boolean()),
        },
    }
}

/// `EventTarget.addEventListener` (DOM §2.7 "add an event listener"): every
/// argument is converted -- `type`, `callback`, then `options` -- before any
/// of the algorithm's own steps run, so a `callback` conversion failure or
/// an `options` dictionary member's own getter throwing both take priority
/// over the "callback is null" no-op. A duplicate `(type, callback,
/// capture)` registration is ignored, whether or not `once`/`passive`
/// match.
pub(crate) fn add_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let key = this_event_target(this, context)?;
    let kind = dom_string(args, 0, context)?;
    let callback = convert_callback(args, context)?;
    let (capture, once, passive) = parse_add_options(context, args)?;
    let Some(callback) = callback else {
        return Ok(JsValue::undefined());
    };
    with_state(context, |s| {
        let list = s.listeners.entry(key).or_default();
        let duplicate = list.iter().any(|l| {
            l.kind == kind && JsObject::equals(&l.callback, &callback) && l.capture == capture
        });
        if !duplicate {
            list.push(Listener {
                kind,
                callback,
                capture,
                once,
                passive,
                is_handler: false,
            });
        }
    })?;
    Ok(JsValue::undefined())
}

/// `EventTarget.removeEventListener` (DOM §2.7 "remove an event listener"):
/// same argument-conversion-first ordering as [`add_event_listener`]. Only
/// `type`/`callback`/`capture` identify the entry to remove --
/// `removeEventListener`'s `options` is `(EventListenerOptions or
/// boolean)`, a dictionary with only a `capture` member, so `once`/
/// `passive` are never read here at all (not merely read and discarded).
pub(crate) fn remove_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let key = this_event_target(this, context)?;
    let kind = dom_string(args, 0, context)?;
    let callback = convert_callback(args, context)?;
    let capture = capture_from_remove_options(context, args)?;
    let Some(callback) = callback else {
        return Ok(JsValue::undefined());
    };
    with_state(context, |s| {
        if let Some(list) = s.listeners.get_mut(&key) {
            list.retain(|l| {
                !(l.kind == kind
                    && JsObject::equals(&l.callback, &callback)
                    && l.capture == capture)
            });
        }
    })?;
    Ok(JsValue::undefined())
}

pub(crate) const EVENT_TARGET_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[],
    methods: &[
        ("addEventListener", 2, add_event_listener),
        ("removeEventListener", 2, remove_event_listener),
        ("dispatchEvent", 1, super::dispatch::dispatch_event),
    ],
};

/// `window`/`self`'s own `addEventListener`/`removeEventListener`/
/// `dispatchEvent`: this runtime's global object has no dedicated
/// `EventTarget` wrapper (`window === globalThis`, an ordinary object), so
/// these are defined directly on it rather than reached through a
/// prototype chain.
pub(crate) fn install_globals(context: &mut Context) -> JsResult<()> {
    let attr = Attribute::WRITABLE | Attribute::CONFIGURABLE;
    let add = function(context, "addEventListener", 2, add_event_listener)?;
    context.register_global_property(js_string!("addEventListener"), add, attr)?;
    let remove = function(context, "removeEventListener", 2, remove_event_listener)?;
    context.register_global_property(js_string!("removeEventListener"), remove, attr)?;
    let dispatch = function(context, "dispatchEvent", 1, super::dispatch::dispatch_event)?;
    context.register_global_property(js_string!("dispatchEvent"), dispatch, attr)?;
    Ok(())
}

#[cfg(test)]
mod tests;
