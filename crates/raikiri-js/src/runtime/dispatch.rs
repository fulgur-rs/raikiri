//! `Event`, `CustomEvent`, `ErrorEvent`, the DOM §2.9 `dispatchEvent`
//! algorithm, event handler IDL attributes (`onload`, `onerror`, ...), and
//! the HTML "report an exception" algorithm.
//!
//! Spec refs:
//! - <https://dom.spec.whatwg.org/#interface-event>
//! - <https://dom.spec.whatwg.org/#interface-customevent>
//! - <https://html.spec.whatwg.org/multipage/webappapis.html#errorevent>
//! - <https://dom.spec.whatwg.org/#dispatching-events>
//! - <https://html.spec.whatwg.org/multipage/webappapis.html#event-handler-idl-attributes>
//! - <https://html.spec.whatwg.org/multipage/webappapis.html#report-the-exception>

use std::cell::{Cell, RefCell};

use boa_engine::object::JsObject;
use boa_engine::object::builtins::JsArray;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{
    Context, Finalize, JsData, JsError, JsNativeError, JsResult, JsString, JsValue, Trace,
};
use raikiri_dom::NodeKind;

use super::event_loop;
use super::events::{self, Listener};
use super::interfaces::{self, Members, protos, wrap};
use super::node::js_str;
use super::webidl::{throw_dom_exception, with_state};

// ---- Native data --------------------------------------------------------

/// Native data of every `Event`/`CustomEvent`/`ErrorEvent` instance. The
/// three interfaces share one data shape rather than three, the same way
/// [`super::geometry::DomRectData`] backs both `DOMRect` and
/// `DOMRectReadOnly`: `CustomEvent`'s `detail` and `ErrorEvent`'s
/// `message`/`filename`/`lineno`/`colno`/`error` are simply unused (left at
/// their default) on a plain `Event`.
///
/// `target`/`currentTarget` reuse the [`super::State::listeners`] key
/// convention (`None` is the window, `Some(index)` a node); the outer
/// `Option` is `None` before the event has ever been dispatched.
#[derive(Debug, Trace, Finalize, JsData)]
pub(crate) struct EventData {
    #[unsafe_ignore_trace]
    kind: String,
    #[unsafe_ignore_trace]
    bubbles: bool,
    #[unsafe_ignore_trace]
    cancelable: bool,
    #[unsafe_ignore_trace]
    composed: bool,
    #[unsafe_ignore_trace]
    is_trusted: bool,
    #[unsafe_ignore_trace]
    time_stamp: f64,
    #[unsafe_ignore_trace]
    target: Cell<Option<Option<usize>>>,
    #[unsafe_ignore_trace]
    current_target: Cell<Option<Option<usize>>>,
    #[unsafe_ignore_trace]
    event_phase: Cell<u16>,
    #[unsafe_ignore_trace]
    stop_propagation: Cell<bool>,
    #[unsafe_ignore_trace]
    stop_immediate_propagation: Cell<bool>,
    #[unsafe_ignore_trace]
    default_prevented: Cell<bool>,
    #[unsafe_ignore_trace]
    in_passive_listener: Cell<bool>,
    #[unsafe_ignore_trace]
    dispatched: Cell<bool>,
    /// The path snapshot taken at the start of the current dispatch (target
    /// first, window last), for `composedPath()`. Empty when not currently
    /// being dispatched.
    #[unsafe_ignore_trace]
    path: RefCell<Vec<Option<usize>>>,
    /// `CustomEvent.detail`.
    detail: JsValue,
    /// `ErrorEvent.message`.
    #[unsafe_ignore_trace]
    message: String,
    /// `ErrorEvent.filename`.
    #[unsafe_ignore_trace]
    filename: String,
    /// `ErrorEvent.lineno`.
    #[unsafe_ignore_trace]
    lineno: u32,
    /// `ErrorEvent.colno`.
    #[unsafe_ignore_trace]
    colno: u32,
    /// `ErrorEvent.error`.
    error: JsValue,
}

fn new_event_data(
    kind: String,
    bubbles: bool,
    cancelable: bool,
    composed: bool,
    is_trusted: bool,
    time_stamp: f64,
) -> EventData {
    EventData {
        kind,
        bubbles,
        cancelable,
        composed,
        is_trusted,
        time_stamp,
        target: Cell::new(None),
        current_target: Cell::new(None),
        event_phase: Cell::new(0),
        stop_propagation: Cell::new(false),
        stop_immediate_propagation: Cell::new(false),
        default_prevented: Cell::new(false),
        in_passive_listener: Cell::new(false),
        dispatched: Cell::new(false),
        path: RefCell::new(Vec::new()),
        detail: JsValue::undefined(),
        message: String::new(),
        filename: String::new(),
        lineno: 0,
        colno: 0,
        error: JsValue::undefined(),
    }
}

/// Read `event`'s native data, or `None` if `event` is not an
/// `Event`/`CustomEvent`/`ErrorEvent` instance. Never holds the object's
/// internal borrow past `f`'s return, so this is always safe to call right
/// before invoking a listener callback.
fn event_data<T>(event: &JsObject, f: impl FnOnce(&EventData) -> T) -> Option<T> {
    event.downcast_ref::<EventData>().map(|d| f(&d))
}

fn type_error(message: impl Into<String>) -> JsError {
    JsNativeError::typ().with_message(message.into()).into()
}

/// `target`/`currentTarget`'s value conversion: the outer `Option` is
/// whether a target has been set at all (`null` if not); `None` inside it
/// is the window, `Some(index)` a node.
fn target_key_to_js(context: &mut Context, key: Option<Option<usize>>) -> JsResult<JsValue> {
    Ok(match key {
        None => JsValue::null(),
        Some(None) => JsValue::from(context.global_object()),
        Some(Some(index)) => wrap(context, index)?.into(),
    })
}

// ---- Dictionary conversions ---------------------------------------------

/// The `EventInit`/`CustomEventInit`/`ErrorEventInit` dictionary argument:
/// `undefined`/absent/`null` all mean "every member at its default"; any
/// other non-object value cannot convert to a dictionary type.
fn dict_arg(args: &[JsValue], i: usize, _context: &mut Context) -> JsResult<Option<JsObject>> {
    match args.get(i) {
        None => Ok(None),
        Some(v) if v.is_undefined() || v.is_null() => Ok(None),
        Some(v) => v
            .as_object()
            .map(Some)
            .ok_or_else(|| type_error("the dictionary argument is not an object")),
    }
}

fn dict_string(
    context: &mut Context,
    dict: &JsObject,
    name: &str,
    default: &str,
) -> JsResult<String> {
    let value = dict.get(JsString::from(name), context)?;
    if value.is_undefined() {
        return Ok(default.to_owned());
    }
    Ok(value.to_string(context)?.to_std_string_escaped())
}

fn dict_u32(context: &mut Context, dict: &JsObject, name: &str, default: u32) -> JsResult<u32> {
    let value = dict.get(JsString::from(name), context)?;
    if value.is_undefined() {
        return Ok(default);
    }
    value.to_u32(context)
}

fn dict_any(context: &mut Context, dict: &JsObject, name: &str) -> JsResult<JsValue> {
    dict.get(JsString::from(name), context)
}

fn event_init(context: &mut Context, dict: Option<&JsObject>) -> JsResult<(bool, bool, bool)> {
    let Some(dict) = dict else {
        return Ok((false, false, false));
    };
    Ok((
        events::dict_flag(context, dict, "bubbles")?,
        events::dict_flag(context, dict, "cancelable")?,
        events::dict_flag(context, dict, "composed")?,
    ))
}

fn require_new(this: &JsValue) -> JsResult<()> {
    if this.is_undefined() {
        return Err(type_error("Constructor requires 'new'"));
    }
    Ok(())
}

fn required_dom_string(args: &[JsValue], i: usize, context: &mut Context) -> JsResult<String> {
    if args.len() <= i {
        return Err(type_error("a required argument is missing"));
    }
    Ok(args[i].to_string(context)?.to_std_string_escaped())
}

fn now(context: &mut Context) -> JsResult<f64> {
    with_state(context, |s| s.event_loop.now)
}

// ---- Constructors --------------------------------------------------------

/// `new Event(type, eventInitDict = {})` (DOM §2.2).
pub(crate) fn event_constructor(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_new(this)?;
    let kind = required_dom_string(args, 0, context)?;
    let dict = dict_arg(args, 1, context)?;
    let (bubbles, cancelable, composed) = event_init(context, dict.as_ref())?;
    let time_stamp = now(context)?;
    let data = new_event_data(kind, bubbles, cancelable, composed, false, time_stamp);
    let proto = protos(context).event.clone();
    Ok(JsObject::from_proto_and_data(Some(proto), data).into())
}

/// `new CustomEvent(type, eventInitDict = {})` (DOM §2.3); `initCustomEvent`
/// is not implemented (out of scope, see the task brief).
pub(crate) fn custom_event_constructor(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_new(this)?;
    let kind = required_dom_string(args, 0, context)?;
    let dict = dict_arg(args, 1, context)?;
    let (bubbles, cancelable, composed) = event_init(context, dict.as_ref())?;
    let detail = match &dict {
        Some(d) => dict_any(context, d, "detail")?,
        None => JsValue::undefined(),
    };
    let time_stamp = now(context)?;
    let mut data = new_event_data(kind, bubbles, cancelable, composed, false, time_stamp);
    data.detail = detail;
    let proto = protos(context).custom_event.clone();
    Ok(JsObject::from_proto_and_data(Some(proto), data).into())
}

/// `new ErrorEvent(type, eventInitDict = {})` (HTML
/// §8.1.7.3.1's `ErrorEvent` interface).
pub(crate) fn error_event_constructor(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_new(this)?;
    let kind = required_dom_string(args, 0, context)?;
    let dict = dict_arg(args, 1, context)?;
    let (bubbles, cancelable, composed) = event_init(context, dict.as_ref())?;
    let (message, filename, lineno, colno, error) = match &dict {
        Some(d) => (
            dict_string(context, d, "message", "")?,
            dict_string(context, d, "filename", "")?,
            dict_u32(context, d, "lineno", 0)?,
            dict_u32(context, d, "colno", 0)?,
            dict_any(context, d, "error")?,
        ),
        None => (String::new(), String::new(), 0, 0, JsValue::undefined()),
    };
    let time_stamp = now(context)?;
    let mut data = new_event_data(kind, bubbles, cancelable, composed, false, time_stamp);
    data.message = message;
    data.filename = filename;
    data.lineno = lineno;
    data.colno = colno;
    data.error = error;
    let proto = protos(context).error_event.clone();
    Ok(JsObject::from_proto_and_data(Some(proto), data).into())
}

// ---- Event / CustomEvent / ErrorEvent members ---------------------------

fn event_type(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| js_str(&d.kind))
}

fn get_target(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = with_event(this, |d| d.target.get())?;
    target_key_to_js(context, key)
}

fn get_current_target(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = with_event(this, |d| d.current_target.get())?;
    target_key_to_js(context, key)
}

fn get_event_phase(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| JsValue::from(d.event_phase.get()))
}

fn get_bubbles(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| JsValue::from(d.bubbles))
}

fn get_cancelable(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| JsValue::from(d.cancelable))
}

fn get_composed(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| JsValue::from(d.composed))
}

fn get_default_prevented(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| JsValue::from(d.default_prevented.get()))
}

fn get_is_trusted(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| JsValue::from(d.is_trusted))
}

fn get_time_stamp(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| JsValue::from(d.time_stamp))
}

fn get_detail(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| d.detail.clone())
}

fn get_message(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| js_str(&d.message))
}

fn get_filename(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| js_str(&d.filename))
}

fn get_lineno(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| JsValue::from(d.lineno))
}

fn get_colno(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| JsValue::from(d.colno))
}

fn get_error(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| d.error.clone())
}

fn stop_propagation(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| d.stop_propagation.set(true))?;
    Ok(JsValue::undefined())
}

fn stop_immediate_propagation(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| {
        d.stop_propagation.set(true);
        d.stop_immediate_propagation.set(true);
    })?;
    Ok(JsValue::undefined())
}

fn prevent_default(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with_event(this, |d| {
        if d.cancelable && !d.in_passive_listener.get() {
            d.default_prevented.set(true);
        }
    })?;
    Ok(JsValue::undefined())
}

fn composed_path(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (dispatched, path) = with_event(this, |d| (d.dispatched.get(), d.path.borrow().clone()))?;
    if !dispatched {
        return Ok(JsArray::from_iter(std::iter::empty(), context).into());
    }
    let mut items = Vec::with_capacity(path.len());
    for key in path {
        items.push(target_key_to_js(context, Some(key))?);
    }
    Ok(JsArray::from_iter(items, context).into())
}

/// Brand-checked native-data access shared by every `Event` (and
/// `CustomEvent`/`ErrorEvent`) getter/method that does not itself need to
/// build a `JsResult<JsValue>` from an `Option<JsObject>` first.
fn with_event<T>(this: &JsValue, f: impl FnOnce(&EventData) -> T) -> JsResult<T> {
    this.as_object()
        .and_then(|o| event_data(&o, f))
        .ok_or_else(|| type_error("'this' is not an Event"))
}

pub(crate) const EVENT_MEMBERS: Members = Members {
    getters: &[
        ("type", event_type),
        ("target", get_target),
        ("currentTarget", get_current_target),
        ("eventPhase", get_event_phase),
        ("bubbles", get_bubbles),
        ("cancelable", get_cancelable),
        ("composed", get_composed),
        ("defaultPrevented", get_default_prevented),
        ("isTrusted", get_is_trusted),
        ("timeStamp", get_time_stamp),
    ],
    accessors: &[],
    methods: &[
        ("stopPropagation", 0, stop_propagation),
        ("stopImmediatePropagation", 0, stop_immediate_propagation),
        ("preventDefault", 0, prevent_default),
        ("composedPath", 0, composed_path),
    ],
};

pub(crate) const CUSTOM_EVENT_MEMBERS: Members = Members {
    getters: &[("detail", get_detail)],
    accessors: &[],
    methods: &[],
};

pub(crate) const ERROR_EVENT_MEMBERS: Members = Members {
    getters: &[
        ("message", get_message),
        ("filename", get_filename),
        ("lineno", get_lineno),
        ("colno", get_colno),
        ("error", get_error),
    ],
    accessors: &[],
    methods: &[],
};

/// `Event.NONE`/`CAPTURING_PHASE`/`AT_TARGET`/`BUBBLING_PHASE`, defined on
/// both `Event` and `Event.prototype` by [`super::interfaces::install`].
pub(crate) const EVENT_PHASE_CONSTANTS: &[(&str, u16)] = &[
    ("NONE", 0),
    ("CAPTURING_PHASE", 1),
    ("AT_TARGET", 2),
    ("BUBBLING_PHASE", 3),
];

const NONE_PHASE: u16 = 0;
const CAPTURING_PHASE: u16 = 1;
const AT_TARGET: u16 = 2;
const BUBBLING_PHASE: u16 = 3;

// ---- DOM §2.9 dispatch ---------------------------------------------------

/// The event path for `target` (`None` = window): `target` itself, then its
/// ancestors (`Document::parent_of`), then, only if that walk actually
/// reaches the document node, one final hop to the window. A detached
/// subtree's topmost node is never the document, so its path never reaches
/// the window, matching real DOM behavior.
fn build_path(context: &mut Context, target: Option<usize>) -> JsResult<Vec<Option<usize>>> {
    let Some(start) = target else {
        return Ok(vec![None]);
    };
    let mut path = vec![Some(start)];
    let mut current = start;
    loop {
        let parent = with_state(context, |s| s.host.document().parent_of(current))?;
        match parent {
            Some(p) => {
                path.push(Some(p));
                current = p;
            }
            None => break,
        }
    }
    let reaches_document = with_state(context, |s| {
        s.host.document().get_node(current).map(|n| n.kind()) == Some(NodeKind::Document)
    })?;
    if reaches_document {
        path.push(None);
    }
    Ok(path)
}

fn set_current_target(event: &JsObject, node: Option<usize>, phase: u16) {
    let _ = event_data(event, |d| {
        d.current_target.set(Some(node));
        d.event_phase.set(phase);
    });
}

fn stopped(event: &JsObject) -> bool {
    event_data(event, |d| d.stop_propagation.get()).unwrap_or(true)
}

fn immediate_stopped(event: &JsObject) -> bool {
    event_data(event, |d| d.stop_immediate_propagation.get()).unwrap_or(true)
}

fn default_prevented(event: &JsObject) -> bool {
    event_data(event, |d| d.default_prevented.get()).unwrap_or(false)
}

fn target_this_value(context: &mut Context, node: Option<usize>) -> JsResult<JsValue> {
    match node {
        None => Ok(JsValue::from(context.global_object())),
        Some(index) => Ok(wrap(context, index)?.into()),
    }
}

/// Call `listener`'s callback (DOM §2.9's "call a user object's
/// operation"): a callable value is called directly with `this_value`; an
/// object that is not itself callable falls back to its own `handleEvent`
/// method, called with the listener object itself as `this`. Neither form
/// exists (a plain, non-callable object with no callable `handleEvent`) is
/// a silent no-op, matching how a non-callable `EventHandler` value is
/// simply never invoked.
fn call_callback(
    context: &mut Context,
    callback: &JsObject,
    event: &JsObject,
    this_value: &JsValue,
) -> JsResult<JsValue> {
    let event_value = JsValue::from(event.clone());
    if callback.is_callable() {
        return callback.call(this_value, &[event_value], context);
    }
    let handle_event = callback.get(JsString::from("handleEvent"), context)?;
    match handle_event.as_callable() {
        Some(handler) => handler.call(&JsValue::from(callback.clone()), &[event_value], context),
        None => Ok(JsValue::undefined()),
    }
}

/// HTML's special "event handler processing algorithm" for `window`'s
/// `onerror` handler: called with `(message, filename, lineno, colno,
/// error)` instead of the `Event`, and a return value that
/// [`boa_engine::JsValue::to_boolean`]s to `true` cancels the event (as if
/// the handler had called `preventDefault()`), rather than requiring the
/// handler to call `preventDefault()` itself.
fn call_window_on_error(
    context: &mut Context,
    callback: &JsObject,
    event: &JsObject,
) -> JsResult<JsValue> {
    let Some((message, filename, lineno, colno, error)) = event_data(event, |d| {
        (
            d.message.clone(),
            d.filename.clone(),
            d.lineno,
            d.colno,
            d.error.clone(),
        )
    }) else {
        return Ok(JsValue::undefined());
    };
    let this_value = JsValue::from(context.global_object());
    let args = [
        js_str(&message),
        js_str(&filename),
        JsValue::from(lineno),
        JsValue::from(colno),
        error,
    ];
    let result = callback.call(&this_value, &args, context)?;
    if result.to_boolean() {
        let _ = event_data(event, |d| {
            if d.cancelable && !d.in_passive_listener.get() {
                d.default_prevented.set(true);
            }
        });
    }
    Ok(result)
}

/// Invoke every listener in `node`'s snapshot whose type matches and whose
/// capture flag passes `capture_filter` (`Some(true)`: capturing phase,
/// ancestors only; `Some(false)`: bubbling phase, ancestors only; `None`:
/// the target phase, every listener regardless of capture). Returns `Err`
/// only for an engine abort (a resource limit hit inside a listener), which
/// must stop dispatch outright rather than being reported and continued.
fn invoke(
    context: &mut Context,
    event: &JsObject,
    node: Option<usize>,
    capture_filter: Option<bool>,
) -> JsResult<()> {
    let kind = event_data(event, |d| d.kind.clone()).unwrap_or_default();
    let snapshot: Vec<Listener> = with_state(context, |s| {
        s.listeners.get(&node).cloned().unwrap_or_default()
    })?;
    let this_value = target_this_value(context, node)?;
    for listener in snapshot {
        if listener.kind != kind {
            continue;
        }
        match capture_filter {
            Some(true) if !listener.capture => continue,
            Some(false) if listener.capture => continue,
            _ => {}
        }
        if listener.once {
            // `node`'s entry necessarily already exists (`listener` itself
            // came from a snapshot of it), but `entry(...).or_default()`
            // -- rather than a defensive `get_mut` -- keeps that invariant
            // from needing its own untestable "what if it doesn't" branch.
            let _ = with_state(context, |s| {
                s.listeners.entry(node).or_default().retain(|l| {
                    !(l.kind == listener.kind
                        && JsObject::equals(&l.callback, &listener.callback)
                        && l.capture == listener.capture)
                });
            });
        }
        if listener.passive {
            let _ = event_data(event, |d| d.in_passive_listener.set(true));
        }
        let result = if listener.is_handler && node.is_none() && kind == "error" {
            call_window_on_error(context, &listener.callback, event)
        } else {
            call_callback(context, &listener.callback, event, &this_value)
        };
        if listener.passive {
            let _ = event_data(event, |d| d.in_passive_listener.set(false));
        }
        if let Err(error) = result {
            if let Some(reason) = event_loop::abort_for(&error) {
                let _ = event_loop::abort(context, reason);
                return Err(error);
            }
            report_exception(context, &error, None);
        }
        if immediate_stopped(event) {
            break;
        }
    }
    Ok(())
}

fn run_dispatch(context: &mut Context, event: &JsObject, path: &[Option<usize>]) -> JsResult<()> {
    for &node in path[1..].iter().rev() {
        set_current_target(event, node, CAPTURING_PHASE);
        invoke(context, event, node, Some(true))?;
        if stopped(event) {
            return Ok(());
        }
    }
    let target_node = path[0];
    set_current_target(event, target_node, AT_TARGET);
    invoke(context, event, target_node, None)?;
    let bubbles = event_data(event, |d| d.bubbles).unwrap_or(false);
    if !bubbles || stopped(event) {
        return Ok(());
    }
    for &node in &path[1..] {
        set_current_target(event, node, BUBBLING_PHASE);
        invoke(context, event, node, Some(false))?;
        if stopped(event) {
            break;
        }
    }
    Ok(())
}

/// DOM §2.9 "to dispatch an event": build the event path, run the
/// capturing/target/bubbling phases, and reset `target`/`eventPhase` at the
/// end. Returns `!defaultPrevented`. `Err` is either an `InvalidStateError`
/// (re-entrant dispatch of the same event) or a propagated engine abort.
pub(crate) fn dispatch(
    context: &mut Context,
    event: &JsObject,
    target: Option<usize>,
) -> JsResult<bool> {
    let Some(already_dispatched) = event_data(event, |d| d.dispatched.get()) else {
        return Err(type_error("'event' is not an Event"));
    };
    if already_dispatched {
        return Err(throw_dom_exception(
            context,
            "InvalidStateError",
            "the event is already being dispatched",
        ));
    }
    let path = build_path(context, target)?;
    let _ = event_data(event, |d| {
        d.dispatched.set(true);
        d.target.set(Some(target));
        d.stop_propagation.set(false);
        d.stop_immediate_propagation.set(false);
        *d.path.borrow_mut() = path.clone();
    });
    let result = run_dispatch(context, event, &path);
    let _ = event_data(event, |d| {
        d.dispatched.set(false);
        d.current_target.set(None);
        d.event_phase.set(NONE_PHASE);
    });
    result?;
    Ok(!default_prevented(event))
}

/// `EventTarget.prototype.dispatchEvent`/`window.dispatchEvent`.
pub(crate) fn dispatch_event(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let key = events::this_event_target(this, context)?;
    let event = args
        .first()
        .and_then(JsValue::as_object)
        .filter(|o| o.is::<EventData>())
        .ok_or_else(|| type_error("parameter 1 is not of type 'Event'"))?;
    let result = dispatch(context, &event, key)?;
    Ok(JsValue::from(result))
}

/// Fire a trusted event of `type` at `target` (`None` = window). Returns
/// `!defaultPrevented`.
#[allow(
    dead_code,
    reason = "produced now for a future document-lifecycle caller (DOMContentLoaded, load, script load/error) that fires trusted events; exercised directly by this module's own tests until that caller exists"
)]
pub(crate) fn fire_event(
    context: &mut Context,
    target: Option<usize>,
    kind: &str,
    bubbles: bool,
    cancelable: bool,
) -> JsResult<bool> {
    let time_stamp = now(context)?;
    let data = new_event_data(
        kind.to_owned(),
        bubbles,
        cancelable,
        false,
        true,
        time_stamp,
    );
    let proto = protos(context).event.clone();
    let event = JsObject::from_proto_and_data(Some(proto), data);
    dispatch(context, &event, target)
}

/// HTML "report an exception": dispatch a trusted, cancelable `ErrorEvent`
/// at `window`. If nothing cancels it (no `onerror` handler returned a
/// truthy value), `error`'s message is recorded in
/// [`super::event_loop::EventLoop::uncaught_errors`]. `source` is the
/// script's location, when known (a future caller's concern; nothing in
/// this runtime yet threads one through).
///
/// A guard prevents unbounded recursion: an exception thrown by a listener
/// while *this* `ErrorEvent` is itself being reported (most commonly the
/// `window.onerror` handler throwing) is recorded directly rather than
/// re-entering this algorithm.
pub(crate) fn report_exception(context: &mut Context, error: &JsError, source: Option<&str>) {
    let message = super::error_message(error, context);
    let already_reporting = with_state(context, |s| {
        std::mem::replace(&mut s.reporting_exception, true)
    })
    .unwrap_or(true);
    if already_reporting {
        let _ = with_state(context, |s| s.event_loop.uncaught_errors.push(message));
        return;
    }
    let error_value = error
        .clone()
        .into_opaque(context)
        .unwrap_or_else(|_| JsValue::undefined());
    let time_stamp = now(context).unwrap_or(0.0);
    let mut data = new_event_data("error".to_owned(), false, true, false, true, time_stamp);
    data.message = message.clone();
    data.filename = source.unwrap_or_default().to_owned();
    data.error = error_value;
    let proto = protos(context).error_event.clone();
    let event = JsObject::from_proto_and_data(Some(proto), data);
    let handled = dispatch(context, &event, None);
    let _ = with_state(context, |s| s.reporting_exception = false);
    if matches!(handled, Ok(true)) {
        let _ = with_state(context, |s| s.event_loop.uncaught_errors.push(message));
    }
}

// ---- Event handler IDL attributes ---------------------------------------

/// `on<kind>` getter (HTML §8.1.7.2 "getting the current value of the event
/// handler"): the callback of the one listener this attribute manages, or
/// `null` if it was never assigned (or was cleared).
fn event_handler_get(this: &JsValue, kind: &str, context: &mut Context) -> JsResult<JsValue> {
    let key = events::this_event_target(this, context)?;
    let callback = with_state(context, |s| {
        s.listeners.get(&key).and_then(|list| {
            list.iter()
                .find(|l| l.kind == kind && l.is_handler)
                .map(|l| l.callback.clone())
        })
    })?;
    Ok(callback.map_or_else(JsValue::null, JsValue::from))
}

/// `on<kind>` setter (HTML §8.1.7.2 "setting an event handler IDL
/// attribute"): a callable value registers or replaces the one listener
/// this attribute manages, in place, so it keeps whatever position it first
/// occupied among the target's other listeners for `kind`; anything else
/// clears it.
fn event_handler_set(
    this: &JsValue,
    args: &[JsValue],
    kind: &str,
    context: &mut Context,
) -> JsResult<JsValue> {
    let key = events::this_event_target(this, context)?;
    let callback = args.first().and_then(JsValue::as_callable);
    with_state(context, |s| {
        let list = s.listeners.entry(key).or_default();
        let existing = list.iter_mut().find(|l| l.kind == kind && l.is_handler);
        match (existing, callback) {
            (Some(listener), Some(callback)) => listener.callback = callback,
            (Some(_), None) => list.retain(|l| !(l.kind == kind && l.is_handler)),
            (None, Some(callback)) => list.push(Listener {
                kind: kind.to_owned(),
                callback,
                capture: false,
                once: false,
                passive: false,
                is_handler: true,
            }),
            (None, None) => {}
        }
    })?;
    Ok(JsValue::undefined())
}

macro_rules! event_handler_pair {
    ($getter:ident, $setter:ident, $kind:literal) => {
        fn $getter(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
            event_handler_get(this, $kind, context)
        }
        fn $setter(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
            event_handler_set(this, args, $kind, context)
        }
    };
}

event_handler_pair!(on_load, set_on_load, "load");
event_handler_pair!(on_error, set_on_error, "error");
event_handler_pair!(on_click, set_on_click, "click");
event_handler_pair!(on_input, set_on_input, "input");
event_handler_pair!(on_change, set_on_change, "change");

/// `onload`/`onerror`/`onclick`/`oninput`/`onchange` on `HTMLElement`.
/// `window` gets the same five, installed by [`install_window_handlers`]
/// (the global object has no dedicated prototype to share these members
/// through, the same reason [`super::events::install_globals`] exists).
pub(crate) const HTML_ELEMENT_HANDLER_MEMBERS: Members = Members {
    getters: &[],
    accessors: &[
        ("onload", on_load, set_on_load),
        ("onerror", on_error, set_on_error),
        ("onclick", on_click, set_on_click),
        ("oninput", on_input, set_on_input),
        ("onchange", on_change, set_on_change),
    ],
    methods: &[],
};

/// Define `window`'s `onload`/`onerror`/`onclick`/`oninput`/`onchange`
/// accessor properties directly on the global object.
pub(crate) fn install_window_handlers(context: &mut Context) -> JsResult<()> {
    let global = context.global_object();
    for &(name, getter_ptr, setter_ptr) in HTML_ELEMENT_HANDLER_MEMBERS.accessors {
        let getter = interfaces::function(context, &format!("get {name}"), 0, getter_ptr)?;
        let setter = interfaces::function(context, &format!("set {name}"), 1, setter_ptr)?;
        let descriptor = PropertyDescriptor::builder()
            .get(getter)
            .set(setter)
            .enumerable(true)
            .configurable(true)
            .build();
        global.define_property_or_throw(JsString::from(name), descriptor, context)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
