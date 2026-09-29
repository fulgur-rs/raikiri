//! A Boa realm whose DOM interfaces are native objects bound to a
//! raikiri-dom document supplied through [`DocumentHost`].

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use boa_engine::object::FunctionObjectBuilder;
use boa_engine::property::{PropertyDescriptor, PropertyKey};
use boa_engine::{
    Context, JsNativeError, JsObject, JsResult, JsString, JsSymbol, JsValue, NativeFunction, Source,
};

pub(crate) mod collections;
pub(crate) mod dispatch;
pub(crate) mod document;
pub(crate) mod event_loop;
pub(crate) mod events;
pub(crate) mod geometry;
pub mod host;
pub(crate) mod indexed;
pub(crate) mod interfaces;
pub(crate) mod node;
pub(crate) mod query;
pub(crate) mod scripts;
pub(crate) mod style;
pub(crate) mod token_list;
pub(crate) mod tree;
pub(crate) mod webidl;
pub(crate) mod window;

#[cfg(test)]
pub(crate) mod test_host;

pub use event_loop::{Abort, Limits, RunOptions};
pub use host::{BoxGeometry, DocumentHost, DomRect, HostError, PositionKind};
pub use scripts::RunReport;

/// Mutable runtime state shared by every native binding.
pub(crate) struct State {
    pub host: Box<dyn DocumentHost>,
    /// Arena index -> wrapper. Strong references: raikiri-dom never frees
    /// arena slots, so wrappers live exactly as long as the runtime.
    pub wrappers: Vec<Option<JsObject>>,
    /// Set by every DOM mutation; cleared by a successful host flush.
    pub dirty: bool,
    /// Monotonic DOM mutation generation, bumped by every mutation alongside
    /// `dirty`. Live collections cache against this, not `dirty`: a
    /// successful host flush clears `dirty` while the DOM stays changed, so
    /// `dirty` alone cannot tell a cached collection it is stale.
    pub generation: u64,
    /// Live-collection tree walks since the runtime was created. Counts cache
    /// misses (fresh walks) only, never cache hits; tests use it to prove
    /// repeated `length`/index accesses do not re-walk.
    #[cfg(test)]
    pub live_walks: usize,
    /// First host failure seen during the current evaluation.
    pub host_failure: Option<String>,
    /// Per-element `style` objects so `el.style === el.style`.
    pub style_objects: HashMap<usize, JsObject>,
    /// Per-element `getComputedStyle` objects so
    /// `getComputedStyle(el) === getComputedStyle(el)`.
    pub computed_style_objects: HashMap<usize, JsObject>,
    /// Per-element `classList` objects so `el.classList === el.classList`.
    pub class_lists: HashMap<usize, JsObject>,
    /// Per-node `childNodes` lists so `n.childNodes === n.childNodes`.
    pub child_node_lists: HashMap<usize, JsObject>,
    /// Per-node `children` collections so `n.children === n.children`.
    pub children_collections: HashMap<usize, JsObject>,
    /// Registered `EventTarget` listeners, keyed by node arena index; `None`
    /// is the window/global object, which has no arena index of its own.
    /// [`dispatch`] both adds and removes entries here (`addEventListener`/
    /// `removeEventListener`) and reads them back to run a target's
    /// listeners when `dispatchEvent` or a trusted event fires.
    pub listeners: HashMap<Option<usize>, Vec<events::Listener>>,
    /// Task and microtask queues, timers, and the virtual clock.
    pub event_loop: event_loop::EventLoop,
    /// Set while [`dispatch::report_exception`] is dispatching its own
    /// `ErrorEvent`, so that an exception thrown by one of *that* event's
    /// listeners (typically `window.onerror`) is recorded directly instead
    /// of re-entering `report_exception` and dispatching another one.
    ///
    /// This guard is a single, runtime-wide flag rather than one scoped to
    /// the specific exception being reported: a listener that dispatches a
    /// *different*, unrelated event while an outer `report_exception` call
    /// is still on the Rust call stack (for example, from inside a
    /// `window.onerror` handler) has any exception of its own recorded
    /// directly too, rather than getting its own `ErrorEvent`. Distinguishing
    /// "nested because of the report we are already handling" from "nested
    /// because of an unrelated dispatch that happens to be in progress"
    /// would need a per-chain guard (a counter or a stack keyed by the
    /// reporting call), not a single flag; this runtime does not implement
    /// that finer distinction.
    pub reporting_exception: bool,
    /// How many nested [`dispatch::dispatch`] calls are on the Rust call
    /// stack right now (a listener that synchronously dispatches another
    /// event, directly or through another listener, back into a target
    /// still being dispatched). Each level consumes native Rust stack
    /// *before* Boa's own [`Limits::max_recursion`] (JS call frames) would
    /// ever trip, so `dispatch` enforces its own, much smaller bound.
    pub dispatch_depth: u32,
    /// `document.readyState`, defaulting to `Complete` (HTML "current
    /// document readiness"). [`DomRuntime::run_document`] drives it through
    /// `Loading`/`Interactive` before returning; a runtime whose caller never
    /// calls `run_document` (evaluating scripts directly through
    /// [`DomRuntime::evaluate`] instead) simply keeps reading `"complete"`.
    pub ready_state: document::ReadyState,
    /// `document.currentScript`: the arena index of the `<script>` element
    /// currently executing, or `None` when no script is (HTML "current
    /// script").
    pub current_script: Option<usize>,
    /// `console.*` calls recorded during evaluation, in call order.
    pub console: Vec<window::ConsoleMessage>,
}

/// Shared handle to [`State`], stored in the Boa context's host data.
#[derive(Clone)]
pub(crate) struct Shared(pub Rc<RefCell<State>>);

/// The runtime state installed in `context` by [`DomRuntime::new`].
pub(crate) fn shared(context: &Context) -> Shared {
    context
        .get_data::<Shared>()
        .cloned()
        .expect("DomRuntime installs its shared state before running scripts")
}

/// The embedder closures [`DomRuntime::evaluate_with_callback`] and
/// [`DomRuntime::run_document_with_callback`] accept.
type CallbackFn = Box<dyn FnMut(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>>;

/// A native Rust callback installed for script to call, owned by
/// [`DomRuntime::evaluate_with_callback`] and
/// [`DomRuntime::run_document_with_callback`].
///
/// Boa's safe native-function constructors only accept `Copy` closures
/// ([`NativeFunction::from_copy_closure`]) or raw function pointers, so an
/// embedder's `FnMut` closure cannot live in the function object itself.
/// Instead the function object holds the [`callback_trampoline`] function
/// pointer, which looks this slot back up from the context's host data on
/// every call. The slot is plain Rust heap (`Rc<RefCell<..>>`), never seen
/// by Boa's garbage collector -- the same way [`State`] already keeps
/// `JsObject` wrappers alive outside the collector -- so capturing script
/// values in the closure is as safe here as it is there.
///
/// Installing a new callback replaces the previous slot: retained
/// references to the old function object then throw (see
/// [`callback_trampoline`]) instead of reaching the new closure.
#[derive(Clone)]
struct CallbackSlot {
    callback: Rc<RefCell<CallbackFn>>,
}

/// The function pointer behind every callback installed by
/// [`DomRuntime::evaluate_with_callback`] and
/// [`DomRuntime::run_document_with_callback`]: looks the embedder's closure
/// back up from the context's host data and calls it.
///
/// A call with no slot installed -- script kept a reference to a one-shot
/// [`DomRuntime::evaluate_with_callback`] function past its evaluation's
/// return, or a newer callback replaced the slot -- throws a `TypeError`
/// instead of touching freed state, and a reentrant call -- the callback
/// running script that calls the same callback again before the outer call
/// returned -- throws a `TypeError` instead of panicking on the slot's
/// `RefCell`.
fn callback_trampoline(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(slot) = context.get_data::<CallbackSlot>().cloned() else {
        return Err(JsNativeError::typ()
            .with_message("native callback is no longer installed")
            .into());
    };
    let mut callback = match slot.callback.try_borrow_mut() {
        Ok(callback) => callback,
        Err(_) => {
            return Err(JsNativeError::typ()
                .with_message("native callback was reentered")
                .into());
        }
    };
    callback(this, args, context)
}

/// Store `callback` in the context's host data and build the script-visible
/// function object for it (named `name`, taking any number of arguments).
/// The caller installs the returned object on the global object and removes
/// both the property and the slot when the callback's scope ends.
fn native_callback_object<F>(context: &mut Context, name: &str, callback: F) -> JsObject
where
    F: FnMut(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue> + 'static,
{
    let slot = CallbackSlot {
        callback: Rc::new(RefCell::new(Box::new(callback) as CallbackFn)),
    };
    let _ = context.insert_data(slot);
    FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(callback_trampoline),
    )
    .name(JsString::from(name))
    .length(0)
    .build()
    .into()
}

/// Install `callback` on the realm's global object under `key` as a
/// non-enumerable, non-writable, configurable property: invisible to
/// `Object.keys` and assignment, removable afterwards.
fn define_callback_property(
    context: &mut Context,
    key: impl Into<PropertyKey>,
    function: JsObject,
) {
    let descriptor = PropertyDescriptor::builder()
        .value(function)
        .writable(false)
        .enumerable(false)
        .configurable(true);
    context
        .global_object()
        .define_property_or_throw(key, descriptor, context)
        .expect("defining a fresh callback property on the realm's global object does not fail");
}

/// Why a script evaluation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// An uncaught JavaScript exception.
    JavaScript(String),
    /// The embedder failed (layout, stylesheet, fragment parse) while the
    /// script ran. Reported instead of any exception it caused.
    Host(String),
    /// A resource limit was reached, now or by an earlier run. The runtime
    /// runs nothing else once this happens.
    Aborted(Abort),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JavaScript(m) => write!(f, "JavaScript error: {m}"),
            Self::Host(m) => write!(f, "host error: {m}"),
            Self::Aborted(reason) => write!(f, "aborted: {reason}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

/// A JavaScript realm with DOM bindings over one [`DocumentHost`].
pub struct DomRuntime {
    context: Context,
    /// [`DomRuntime::run_document`]'s cached result, once it has been
    /// called: `run_document` is idempotent, so a second call returns this
    /// instead of running the document again.
    report: Option<RunReport>,
}

impl DomRuntime {
    /// Create a realm, register DOM interfaces, and expose `window`,
    /// `self`, and `document`, with the default [`RunOptions`].
    pub fn new<H: DocumentHost>(host: H) -> Result<Self, RuntimeError> {
        Self::with_options(host, RunOptions::default())
    }

    /// [`DomRuntime::new`] with explicit resource limits.
    pub fn with_options<H: DocumentHost>(
        host: H,
        options: RunOptions,
    ) -> Result<Self, RuntimeError> {
        let limits = options.limits;
        let node_count = host.document().node_count();
        let state = State {
            host: Box::new(host),
            wrappers: vec![None; node_count],
            dirty: true,
            generation: 0,
            #[cfg(test)]
            live_walks: 0,
            host_failure: None,
            style_objects: HashMap::new(),
            computed_style_objects: HashMap::new(),
            class_lists: HashMap::new(),
            child_node_lists: HashMap::new(),
            children_collections: HashMap::new(),
            listeners: HashMap::new(),
            event_loop: event_loop::EventLoop::new(limits.clone()),
            reporting_exception: false,
            dispatch_depth: 0,
            ready_state: document::ReadyState::default(),
            current_script: None,
            console: Vec::new(),
        };
        let executor = Rc::new(event_loop::RaikiriJobExecutor);
        let built = Context::builder().job_executor(executor).build();
        // cov:ignore: building fails only for a context that can block while
        // another context is active, and this builder never asks to block.
        let Ok(mut context) = built else {
            return Err(RuntimeError::JavaScript(
                "could not create a realm".to_owned(),
            ));
        };
        let runtime_limits = context.runtime_limits_mut();
        runtime_limits.set_loop_iteration_limit(limits.max_loop_iterations);
        runtime_limits.set_recursion_limit(limits.max_recursion);
        context.insert_data(Shared(Rc::new(RefCell::new(state))));
        // cov:ignore: install only defines properties on a fresh realm's global
        // object and the interface prototypes it just created, which Boa never rejects.
        if let Err(error) = interfaces::install(&mut context) {
            return Err(match error_message(&error, &mut context) {
                Ok(message) => RuntimeError::JavaScript(message),
                // cov:ignore: `install` throws no thrown object with a
                // `toString` that could abort; unreachable at construction
                // time, before any user script or thrown value exists.
                Err(reason) => RuntimeError::Aborted(reason),
            });
        }
        Ok(Self {
            context,
            report: None,
        })
    }

    /// Evaluate a classic script in the global scope.
    ///
    /// An uncaught exception is [`RuntimeError::JavaScript`]. If a host
    /// failure was recorded since the previous `evaluate` -- while this
    /// script or its microtasks ran, earlier by a callback that
    /// [`DomRuntime::run_until_idle`] invoked, such as a timer, or directly
    /// between evaluations through [`DomRuntime::context_mut`] --
    /// [`RuntimeError::Host`] is returned instead, whether or not the script
    /// caught the exception.
    ///
    /// A microtask checkpoint follows the script, as after any script in a
    /// page, so promise reactions it queued have run on return. A resource
    /// limit reached by the script or the checkpoint is
    /// [`RuntimeError::Aborted`], which script cannot catch; after that the
    /// runtime refuses to evaluate anything else.
    pub fn evaluate(&mut self, source: &str) -> Result<JsValue, RuntimeError> {
        if let Some(reason) = event_loop::aborted(&mut self.context) {
            return Err(RuntimeError::Aborted(reason));
        }
        let result = self.context.eval(Source::from_bytes(source));
        let checkpoint = match &result {
            Err(error) => match event_loop::abort_for(error) {
                Some(reason) => Err(event_loop::abort(&mut self.context, reason)),
                None => event_loop::microtask_checkpoint(&mut self.context),
            },
            Ok(_) => event_loop::microtask_checkpoint(&mut self.context),
        };
        // Map the thrown value before reading the host-failure slot:
        // stringifying runs script (a custom toString or an Error message
        // getter) which can itself record a host failure. Reading the slot
        // first would leave that failure for the next call to report.
        let mapped = result.map_err(|error| match error_message(&error, &mut self.context) {
            Ok(message) => RuntimeError::JavaScript(message),
            Err(reason) => RuntimeError::Aborted(reason),
        });
        let failure = webidl::take_host_failure(&mut self.context);
        if let Err(reason) = checkpoint {
            return Err(RuntimeError::Aborted(reason));
        }
        // A limit hit while stringifying outranks a host failure, the same
        // as the checkpoint above does.
        if let Err(RuntimeError::Aborted(_)) = &mapped {
            return mapped;
        }
        if let Some(message) = failure {
            return Err(RuntimeError::Host(message));
        }
        mapped
    }

    /// Evaluate a classic script with a native Rust callback visible to it
    /// under `name`, then remove `name` again.
    ///
    /// This is the typed alternative to reaching for
    /// [`DomRuntime::context_mut`] and building the function object by hand
    /// with `boa_engine` directly: the callback is installed as a
    /// non-enumerable, non-writable, configurable global just before
    /// evaluation and deleted just after, whether evaluation succeeded or
    /// not, so a later script never sees it. A script that keeps its own
    /// reference to the function and calls it afterwards gets a `TypeError`.
    /// Otherwise evaluation works exactly like [`DomRuntime::evaluate`],
    /// including the microtask checkpoint and the [`RuntimeError::Host`] /
    /// [`RuntimeError::Aborted`] precedence.
    ///
    /// `name` must not already exist as an own property of the global
    /// object: overwriting (and then deleting) a page's own global would
    /// silently clobber it, so that is [`RuntimeError::JavaScript`] instead.
    /// Installing replaces any callback an earlier call left installed (see
    /// [`DomRuntime::run_document_with_callback`]). The callback may capture
    /// Rust state (`FnMut`); it must not call itself reentrantly (a callback
    /// that runs script which calls the same callback again throws a
    /// `TypeError` instead of panicking).
    pub fn evaluate_with_callback<F>(
        &mut self,
        source: &str,
        name: &str,
        callback: F,
    ) -> Result<JsValue, RuntimeError>
    where
        F: FnMut(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue> + 'static,
    {
        let key = JsString::from(name);
        let defined = self
            .context
            .global_object()
            .has_own_property(key.clone(), &mut self.context)
            .expect("reading an own property of the realm's global object does not fail");
        if defined {
            return Err(RuntimeError::JavaScript(format!(
                "callback name {name:?} is already defined on the global object"
            )));
        }
        let function = native_callback_object(&mut self.context, name, callback);
        define_callback_property(&mut self.context, key.clone(), function);
        let result = self.evaluate(source);
        let _ = self
            .context
            .global_object()
            .delete_property_or_throw(key, &mut self.context);
        self.context.remove_data::<CallbackSlot>();
        result
    }

    /// Run tasks and microtasks until both queues are empty or a limit is hit.
    ///
    /// Each turn takes the task due earliest (registration order breaks
    /// ties), advances the virtual clock to its due time, runs it, and then
    /// performs a microtask checkpoint. Reaching a limit discards every
    /// queue; the runtime then refuses to run anything else, and later
    /// calls return the same [`Abort`].
    ///
    /// [`Limits`] does not bound everything: native builtin loops,
    /// regular-expression backtracking, and loop-free recursive call trees
    /// can use unbounded CPU, deeply nested source can overflow the native
    /// stack in Boa's parser, and there is no heap bound (see [`Limits`]).
    /// Untrusted content needs an external watchdog as well.
    pub fn run_until_idle(&mut self) -> Result<(), Abort> {
        event_loop::run_until_idle(&mut self.context)
    }

    /// Run the document end to end (HTML's script processing model,
    /// <https://html.spec.whatwg.org/multipage/scripting.html>, run as a
    /// single batch after parsing rather than interleaved with it -- see
    /// [`raikiri_traits::ScriptExecutor`]'s own doc comment): collect every
    /// classic `<script>` element in tree order and run it, fire
    /// `DOMContentLoaded`, drain the event loop, fire `load`, and drain it
    /// again. Idempotent: a second call does nothing and returns the same
    /// [`RunReport`] as the first.
    ///
    /// `async`/`defer` are not distinguished: every classic script runs in
    /// tree order as if neither attribute were present. This is an
    /// approximation of the real scheduling those attributes specify, not a
    /// full implementation of it. A `<script>` element inserted by an
    /// earlier script while this run is already in progress is not picked
    /// up by it either: the script list is collected once, up front.
    pub fn run_document(&mut self) -> RunReport {
        if let Some(report) = &self.report {
            return report.clone();
        }
        let report = scripts::run(self);
        self.report = Some(report.clone());
        report
    }

    /// Run the document end to end like [`DomRuntime::run_document`], with
    /// a native Rust callback installed for the whole run under a fresh
    /// symbol, then remove the symbol again.
    ///
    /// This is the [`DomRuntime::run_document`] counterpart of
    /// [`DomRuntime::evaluate_with_callback`], for harnesses whose delivery
    /// script runs mid-document as one of the page's own `<script>` elements
    /// rather than as a single evaluated string: `symbol_description` is the
    /// fresh symbol's description, so the delivery script finds it by
    /// scanning `Object.getOwnPropertySymbols(globalThis)` for that
    /// description, claims it (and deletes it) for itself. A page script
    /// running before the delivery script can observe the symbol the same
    /// way; that window is inherent to handing a value to mid-document
    /// scripts through the global object. If no script claims it, this
    /// removes it after the run instead, so no property leaks either way.
    ///
    /// The callback slot itself stays installed after the run, so a delivery
    /// script that kept its own reference (for example, a testharness
    /// completion callback the timeout probe triggers after the run) can
    /// still deliver; only the global-object property is removed. A later
    /// callback install replaces the slot, and teardown drops it with the
    /// runtime.
    ///
    /// If [`DomRuntime::run_document`] already ran, this returns its cached
    /// [`RunReport`] without invoking the callback, the same idempotency
    /// [`DomRuntime::run_document`] itself has. The callback's reentrancy
    /// rule is the same as [`DomRuntime::evaluate_with_callback`]'s.
    pub fn run_document_with_callback<F>(
        &mut self,
        symbol_description: &str,
        callback: F,
    ) -> RunReport
    where
        F: FnMut(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue> + 'static,
    {
        // Idempotent like `run_document`: a cached report means the
        // document already ran, so there is nothing to hand the callback to.
        if self.report.is_some() {
            return self.run_document();
        }
        let key = JsSymbol::new(Some(JsString::from(symbol_description)))
            .expect("symbol ids run out only after 2^64 symbols");
        let function = native_callback_object(&mut self.context, symbol_description, callback);
        define_callback_property(&mut self.context, key.clone(), function);
        let report = self.run_document();
        let _ = self
            .context
            .global_object()
            .delete_property_or_throw(key, &mut self.context);
        // The slot stays: a delivery script that kept its own reference
        // still delivers afterwards (see the method doc comment). The
        // property above is what must not leak, and it is gone.
        report
    }

    /// The virtual clock, in milliseconds since the runtime was created.
    pub fn now(&self) -> f64 {
        shared(&self.context).0.borrow().event_loop.now
    }

    /// The underlying Boa context, for harness adapters that need raw
    /// engine access (for example, reading back context host data).
    ///
    /// A host failure recorded through the context between evaluations (for
    /// example, by driving Boa directly instead of through
    /// [`DomRuntime::evaluate`]) stays parked in the runtime state and
    /// surfaces as [`RuntimeError::Host`] on the next [`DomRuntime::evaluate`]
    /// call, the same as a failure recorded while a script ran.
    ///
    /// To pass a native Rust callback into evaluated code, prefer
    /// [`DomRuntime::evaluate_with_callback`] or
    /// [`DomRuntime::run_document_with_callback`]: they install the function
    /// object, hide it from string-key enumeration, and remove it again,
    /// without the caller touching `boa_engine`'s function, symbol, or
    /// property machinery directly.
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    /// Tear down the realm and return the host with its (mutated) document.
    ///
    /// Recover the concrete host with the [`DocumentHost`] downcast helpers
    /// (`Box<dyn DocumentHost>::downcast`) or directly with
    /// [`DomRuntime::try_into_host`].
    pub fn into_host(mut self) -> Box<dyn DocumentHost> {
        let shared = self
            .context
            .remove_data::<Shared>()
            .expect("shared state is installed for the runtime's lifetime");
        let _ = self.context.remove_data::<interfaces::Protos>();
        drop(self.context);
        let state = Rc::try_unwrap(shared.0)
            .unwrap_or_else(|_| unreachable!("bindings hold no Shared clones outside the context")) // cov:ignore: the context, the only other owner, was dropped above
            .into_inner();
        state.host
    }

    /// Tear down the realm and recover the concrete host it was created
    /// with, with its (mutated) document.
    ///
    /// On a type mismatch nothing is torn down: the runtime is returned
    /// untouched as `Err`, so the caller can keep evaluating or try another
    /// concrete type. A state borrow held elsewhere (for example, a binding
    /// still on the Rust call stack) also returns `Err` rather than
    /// panicking.
    ///
    /// The `Err` payload is the runtime itself by design (nothing is torn
    /// down on mismatch), so the large-error-variant lint does not apply.
    #[allow(clippy::result_large_err)]
    pub fn try_into_host<H: DocumentHost>(self) -> Result<H, Self> {
        let matches = shared(&self.context)
            .0
            .try_borrow()
            .is_ok_and(|state| state.host.as_any().is::<H>());
        if !matches {
            return Err(self);
        }
        let boxed = self.into_host();
        let any: Box<dyn Any> = boxed.into_any();
        any.downcast::<H>().map(|host| *host).map_err(|_| {
            // cov:ignore: the type check above rules a mismatch out, so the
            // `Any` downcast cannot fail here.
            unreachable!("DocumentHost type check passed but Any downcast failed")
        })
    }
}

/// A readable message for an uncaught exception. Native errors (including
/// ones already thrown into script as `TypeError` objects and so on) render
/// as `Name: message`. A thrown object that Boa does not recognize as one of
/// its own native errors -- notably a `DOMException`, whose name/message
/// live in Rust-side object data rather than in Boa's error representation --
/// runs `Error.prototype.toString` instead, which resolves the same
/// `Name: message` form through its `name`/`message` accessors; anything
/// else falls back to its own string form.
///
/// Running that `toString` can itself run arbitrary script (a user-defined
/// `toString` method, e.g. `throw { toString(){ while (true) {} } }`), which
/// can hit a resource limit. That case is `Err`, and the abort is already
/// recorded (sticky) by the time this returns; a limit hit by the *original*
/// exception's own construction was already caught before that exception
/// ever reached here (see [`event_loop::abort_for`] at every call site that
/// produced it).
pub(crate) fn error_message(
    error: &boa_engine::JsError,
    context: &mut Context,
) -> Result<String, Abort> {
    if let Ok(native) = error.try_native(context) {
        return Ok(native.to_string());
    }
    if let Some(value) = error.as_opaque()
        && value.is_object()
    {
        let to_string_result = value.to_string(context);
        if let Ok(message) = &to_string_result {
            return Ok(message.to_std_string_escaped());
        }
        if let Err(to_string_error) = &to_string_result
            && let Some(reason) = event_loop::abort_for(to_string_error)
        {
            return Err(event_loop::abort(context, reason));
        }
        // An ordinary (non-abort) exception from `toString` itself: fall
        // through to the outer error's own `Display` form below, same as
        // when the thrown value isn't an object at all.
    }
    Ok(error.to_string())
}

#[cfg(test)]
mod tests;
