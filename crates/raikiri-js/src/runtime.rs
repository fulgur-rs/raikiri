//! A Boa realm whose DOM interfaces are native objects bound to a
//! raikiri-dom document supplied through [`DocumentHost`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use boa_engine::{Context, JsObject, JsValue, Source};

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
pub(crate) mod style;
pub(crate) mod token_list;
pub(crate) mod tree;
pub(crate) mod webidl;

#[cfg(test)]
pub(crate) mod test_host;

pub use event_loop::{Abort, Limits, RunOptions};
pub use host::{BoxGeometry, DocumentHost, DomRect, HostError, PositionKind};

/// Mutable runtime state shared by every native binding.
pub(crate) struct State {
    pub host: Box<dyn DocumentHost>,
    /// Arena index -> wrapper. Strong references: raikiri-dom never frees
    /// arena slots, so wrappers live exactly as long as the runtime.
    pub wrappers: Vec<Option<JsObject>>,
    /// Set by every DOM mutation; cleared by a successful host flush.
    pub dirty: bool,
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
    /// Dispatch is out of scope for this runtime -- only registration state
    /// is kept.
    pub listeners: HashMap<Option<usize>, Vec<events::Listener>>,
    /// Task and microtask queues, timers, and the virtual clock.
    pub event_loop: event_loop::EventLoop,
    /// Set while [`dispatch::report_exception`] is dispatching its own
    /// `ErrorEvent`, so that an exception thrown by one of *that* event's
    /// listeners (typically `window.onerror`) is recorded directly instead
    /// of re-entering `report_exception` and dispatching another one.
    pub reporting_exception: bool,
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
            host_failure: None,
            style_objects: HashMap::new(),
            computed_style_objects: HashMap::new(),
            class_lists: HashMap::new(),
            child_node_lists: HashMap::new(),
            children_collections: HashMap::new(),
            listeners: HashMap::new(),
            event_loop: event_loop::EventLoop::new(limits.clone()),
            reporting_exception: false,
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
            return Err(RuntimeError::JavaScript(error_message(
                &error,
                &mut context,
            )));
        }
        Ok(Self { context })
    }

    /// Evaluate a classic script in the global scope.
    ///
    /// An uncaught exception is [`RuntimeError::JavaScript`]. If a host
    /// failure was recorded since the previous `evaluate` -- while this
    /// script or its microtasks ran, or earlier by a callback that
    /// [`DomRuntime::run_until_idle`] invoked, such as a timer --
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
        let deferred = self
            .context
            .remove_data::<webidl::DeferredHostFailure>()
            .map(|f| f.0);
        let recorded = shared(&self.context).0.borrow_mut().host_failure.take();
        let failure = recorded.or(deferred);
        if let Err(reason) = checkpoint {
            return Err(RuntimeError::Aborted(reason));
        }
        if let Some(message) = failure {
            return Err(RuntimeError::Host(message));
        }
        result.map_err(|error| RuntimeError::JavaScript(error_message(&error, &mut self.context)))
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

    /// The virtual clock, in milliseconds since the runtime was created.
    pub fn now(&self) -> f64 {
        shared(&self.context).0.borrow().event_loop.now
    }

    /// The underlying Boa context, for harness adapters.
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    /// Tear down the realm and return the host with its (mutated) document.
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
}

/// A readable message for an uncaught exception. Native errors (including
/// ones already thrown into script as `TypeError` objects and so on) render
/// as `Name: message`. A thrown object that Boa does not recognize as one of
/// its own native errors -- notably a `DOMException`, whose name/message
/// live in Rust-side object data rather than in Boa's error representation --
/// runs `Error.prototype.toString` instead, which resolves the same
/// `Name: message` form through its `name`/`message` accessors; anything
/// else falls back to its own string form.
pub(crate) fn error_message(error: &boa_engine::JsError, context: &mut Context) -> String {
    if let Ok(native) = error.try_native(context) {
        return native.to_string();
    }
    if let Some(value) = error.as_opaque()
        && value.is_object()
        && let Ok(message) = value.to_string(context)
    {
        return message.to_std_string_escaped();
    }
    error.to_string()
}

#[cfg(test)]
mod tests;
