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

/// One registration, including the DOM removed flag. Its slot is stable
/// throughout every active invocation on the owning listener list.
#[derive(Clone, boa_engine::Trace, boa_engine::Finalize)]
pub(crate) struct Listener {
    pub removed: bool,
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

/// Dense registrations with invocation-scoped slot identities.
///
/// While any snapshot is active, entries may only be appended or marked
/// removed. No slot is erased, reordered, or reused until the outermost
/// invocation finishes. Integer snapshots stay private to their owning list;
/// the rooted list traces callbacks even after removal during dispatch.
#[derive(Default, boa_engine::Trace, boa_engine::Finalize)]
pub(super) struct ListenerList {
    entries: boa_gc::GcRefCell<Vec<Listener>>,
    depth: std::cell::Cell<usize>,
    dirty: std::cell::Cell<bool>,
}
impl ListenerList {
    pub(super) fn add(&self, listener: Listener) {
        let mut entries = self.entries.borrow_mut();
        if !entries.iter().any(|l| {
            !l.removed
                && !l.is_handler
                && l.kind == listener.kind
                && l.capture == listener.capture
                && JsObject::equals(&l.callback, &listener.callback)
        }) {
            entries.push(listener);
        }
    }
    pub(super) fn remove(&self, kind: &str, callback: &JsObject, capture: bool) {
        for listener in self.entries.borrow_mut().iter_mut() {
            if !listener.removed
                && !listener.is_handler
                && listener.kind == kind
                && listener.capture == capture
                && JsObject::equals(&listener.callback, callback)
            {
                listener.removed = true;
                self.dirty.set(true);
            }
        }
        self.compact();
    }
    pub(super) fn handler(&self, kind: &str) -> Option<JsObject> {
        self.entries
            .borrow()
            .iter()
            .find(|l| !l.removed && l.is_handler && l.kind == kind)
            .map(|l| l.callback.clone())
    }
    pub(super) fn set_handler(&self, kind: &str, callback: Option<JsObject>) {
        let mut entries = self.entries.borrow_mut();
        let existing = entries
            .iter_mut()
            .find(|l| !l.removed && l.is_handler && l.kind == kind);
        match (existing, callback) {
            (Some(listener), Some(callback)) => listener.callback = callback,
            (Some(listener), None) => {
                listener.removed = true;
                self.dirty.set(true);
            }
            (None, Some(callback)) => entries.push(Listener {
                removed: false,
                kind: kind.to_owned(),
                callback,
                capture: false,
                once: false,
                passive: false,
                is_handler: true,
            }),
            (None, None) => {}
        }
        drop(entries);
        self.compact();
    }
    pub(super) fn begin<const COUNT: bool>(
        &self,
        kind: &str,
        capture: Option<bool>,
        work: &mut crate::listener_diagnostics::Work,
    ) -> Vec<usize> {
        self.depth.set(self.depth.get() + 1);
        let entries = self.entries.borrow();
        if COUNT {
            work.snapshot_candidates += entries.len() as u64;
        }
        let snapshot: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.removed && l.kind == kind && capture.is_none_or(|c| c == l.capture))
            .map(|(index, _)| index)
            .collect();
        if COUNT {
            work.snapshot_entries += snapshot.len() as u64;
        }
        snapshot
    }
    pub(super) fn current(&self, index: usize) -> Option<(JsObject, bool, bool)> {
        let mut entries = self.entries.borrow_mut();
        let listener = &mut entries[index];
        if listener.removed {
            return None;
        }
        if listener.once {
            listener.removed = true;
            self.dirty.set(true);
        }
        Some((
            listener.callback.clone(),
            listener.passive,
            listener.is_handler,
        ))
    }
    pub(super) fn finish(&self) {
        self.depth.set(self.depth.get() - 1);
        self.compact();
    }
    fn compact(&self) {
        if self.depth.get() == 0 && self.dirty.replace(false) {
            let mut entries = self.entries.borrow_mut();
            crate::listener_diagnostics::compaction(entries.len());
            entries.retain(|listener| !listener.removed);
        }
    }
    #[cfg(test)]
    pub(super) fn live_len(&self) -> usize {
        self.entries.borrow().iter().filter(|l| !l.removed).count()
    }
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
/// match -- except against the one listener slot an event handler IDL
/// attribute (`onclick`, ...) manages: that slot's internal callback is
/// never the same identity as one supplied to `addEventListener`, even when
/// both happen to be the very same JS function, so it is never treated as
/// a duplicate of it (nor removable by `removeEventListener`, see below).
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
        s.ensure_listeners(key).add(Listener {
            removed: false,
            kind,
            callback,
            capture,
            once,
            passive,
            is_handler: false,
        })
    })?;
    Ok(JsValue::undefined())
}

/// `EventTarget.removeEventListener` (DOM §2.7 "remove an event listener"):
/// same argument-conversion-first ordering as [`add_event_listener`]. Only
/// `type`/`callback`/`capture` identify the entry to remove --
/// `removeEventListener`'s `options` is `(EventListenerOptions or
/// boolean)`, a dictionary with only a `capture` member, so `once`/
/// `passive` are never read here at all (not merely read and discarded).
/// An event handler IDL attribute's own listener slot is never removed
/// this way, matching [`add_event_listener`]'s own note on why the two
/// never collide.
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
        if let Some(list) = s.listeners(key) {
            list.remove(&kind, &callback, capture);
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
