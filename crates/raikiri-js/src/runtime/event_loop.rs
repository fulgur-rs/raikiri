//! The runtime's event loop on a virtual clock: a microtask queue fed by
//! Boa's promise jobs and `queueMicrotask`, one task queue ordered by
//! (virtual due time, registration order), HTML timers
//! (`setTimeout`/`setInterval`), animation frames, and resource limits.
//!
//! Time never comes from the host: [`EventLoop::now`] starts at 0 and only
//! moves forward when the loop takes a task whose due time is later.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashMap, VecDeque};
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context as TaskContext, Poll, Waker};

use boa_engine::error::{EngineError, RuntimeLimitError};
use boa_engine::job::{GenericJob, Job, JobExecutor, NativeAsyncJob, NativeJob, PromiseJob};
use boa_engine::object::ObjectInitializer;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{
    Context, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Source,
};

use super::error_message;
use super::interfaces::function;
use super::webidl::with_state;

/// Resource limits for one runtime. Reaching any of them aborts the run:
/// the running script stops, every queue is discarded, and the runtime
/// refuses to run anything afterwards.
///
/// # What these limits do not bound
///
/// - CPU time. Boa counts loop iterations per call frame, and native
///   builtins loop without counting, so native loops over huge array-likes
///   (`Array.prototype.forEach.call({length: 1e15}, f)`,
///   `Array.from({length: 1e9})`, `fill`), regular-expression backtracking,
///   and loop-free recursive call trees run unbounded. An embedder that
///   must bound CPU for untrusted content needs an external watchdog.
/// - Parser depth. Deeply nested source (hundreds of nested brackets) can
///   overflow the native stack inside Boa's parser before any limit here
///   applies.
/// - Memory. There is no heap bound.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    /// The latest virtual time, in milliseconds, a task may be due at.
    pub max_virtual_time_ms: f64,
    /// How many tasks and microtasks may run in total. Microtasks count
    /// too, so that an endless `queueMicrotask`/promise chain -- which no
    /// per-frame engine limit sees -- is bounded as well.
    pub max_tasks: u64,
    /// Boa's per-frame loop iteration limit.
    pub max_loop_iterations: u64,
    /// Boa's call recursion limit. Native bindings that call back into
    /// script re-enter on the Rust stack, so this stays near Boa's own
    /// default rather than something much larger.
    pub max_recursion: usize,
    /// The most DOM nodes a document may hold.
    pub max_nodes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_virtual_time_ms: 30_000.0,
            max_tasks: 100_000,
            max_loop_iterations: 10_000_000,
            max_recursion: 512,
            max_nodes: 1_000_000,
        }
    }
}

/// Options for [`super::DomRuntime::with_options`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunOptions {
    /// Resource limits.
    pub limits: Limits,
}

/// Which resource limit stopped the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Abort {
    /// A task was due after [`Limits::max_virtual_time_ms`].
    VirtualTime,
    /// More than [`Limits::max_tasks`] tasks and microtasks.
    Tasks,
    /// Boa's loop iteration limit ([`Limits::max_loop_iterations`]).
    LoopIterations,
    /// Boa's recursion or stack size limit ([`Limits::max_recursion`]).
    Recursion,
    /// More than [`Limits::max_nodes`] DOM nodes.
    Nodes,
}

impl std::fmt::Display for Abort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::VirtualTime => "virtual time limit reached",
            Self::Tasks => "task limit reached",
            Self::LoopIterations => "loop iteration limit reached",
            Self::Recursion => "recursion limit reached",
            Self::Nodes => "node limit reached",
        })
    }
}

/// The [`Abort`] behind an engine error, if `error` is one. Boa raises its
/// runtime limits as engine errors, which script `try`/`catch`, `finally`,
/// and promise rejection handlers never observe: the VM unwinds straight to
/// the embedder. Every place this runtime would otherwise record an error
/// and carry on checks this first.
///
/// Boa's recursion and stack size limits both map to [`Abort::Recursion`].
/// So does an internal engine panic ([`EngineError::Panic`]), which is not
/// a resource limit at all: it is uncatchable in the same way, and stopping
/// the run is the only safe response, so it reuses that variant rather than
/// adding one for a condition script cannot provoke by design.
pub(crate) fn abort_for(error: &JsError) -> Option<Abort> {
    match error.as_engine()? {
        EngineError::RuntimeLimit(RuntimeLimitError::LoopIteration) => Some(Abort::LoopIterations),
        // The recursion and stack size limits both stop runaway recursion.
        // Any other engine error (an internal engine panic) cannot be
        // caught by script either, so it stops the run the same way.
        _ => Some(Abort::Recursion),
    }
}

/// A timer's handler: a callback, or source text stringified when the timer
/// was set (HTML §8.6 timer initialization steps, `TimerHandler`).
enum Handler {
    Callback(JsObject),
    Code(String),
}

struct Timer {
    handler: Handler,
    args: Vec<JsValue>,
    timeout: i32,
    repeat: bool,
}

enum Task {
    /// A timer's task; a timer cleared before it runs has no entry in
    /// [`EventLoop::timers`] and is skipped without counting.
    Timer { id: i32, nesting: u32 },
    /// Run every animation frame callback registered before the frame.
    AnimationFrame,
    /// A native job, such as a Boa timeout job.
    Native(NativeJob),
}

struct Queued {
    due: f64,
    seq: u64,
    task: Task,
}

impl PartialEq for Queued {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Queued {}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Queued {
    /// Reversed, so that [`BinaryHeap`] pops the earliest due time first
    /// and, among equal due times, the earliest registration.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .due
            .total_cmp(&self.due)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

enum Microtask {
    Promise(PromiseJob),
    Generic(GenericJob),
    Async(NativeAsyncJob),
    Callback(JsObject),
}

/// Event loop state, kept in [`super::State`].
pub(crate) struct EventLoop {
    pub limits: Limits,
    /// Virtual time in milliseconds.
    pub now: f64,
    next_seq: u64,
    tasks: BinaryHeap<Queued>,
    microtasks: VecDeque<Microtask>,
    /// The map of active timers, by id.
    timers: HashMap<i32, Timer>,
    next_timer_id: i32,
    /// Animation frame callbacks by handle; handles only grow, so key order
    /// is registration order.
    frame_callbacks: BTreeMap<u32, JsObject>,
    next_frame_handle: u32,
    frame_scheduled: bool,
    /// The timer nesting level of the timer task currently running.
    current_nesting: Option<u32>,
    /// Tasks and microtasks run so far.
    jobs_run: u64,
    /// Set once a limit is reached; the runtime then refuses to run
    /// anything else.
    pub aborted: Option<Abort>,
    /// Messages of exceptions thrown by callbacks the loop invoked (timers,
    /// animation frames, microtasks) that nothing caught.
    pub uncaught_errors: Vec<String>,
}

enum Next {
    Idle,
    Run(Task),
    Abort(Abort),
}

impl EventLoop {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            now: 0.0,
            next_seq: 0,
            tasks: BinaryHeap::new(),
            microtasks: VecDeque::new(),
            timers: HashMap::new(),
            next_timer_id: 1,
            frame_callbacks: BTreeMap::new(),
            next_frame_handle: 1,
            frame_scheduled: false,
            current_nesting: None,
            jobs_run: 0,
            aborted: None,
            uncaught_errors: Vec::new(),
        }
    }

    fn queue_task(&mut self, delay: f64, task: Task) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.tasks.push(Queued {
            due: self.now + delay,
            seq,
            task,
        });
    }

    /// HTML §8.6 timer initialization steps from the nesting level on:
    /// the negative-to-zero and nesting clamps, and queueing the task.
    fn schedule_timer(&mut self, id: i32, timeout: i32) {
        let nesting = self.current_nesting.unwrap_or(0);
        let mut timeout = timeout.max(0);
        if nesting > 5 && timeout < 4 {
            timeout = 4;
        }
        self.queue_task(
            f64::from(timeout),
            Task::Timer {
                id,
                nesting: nesting + 1,
            },
        );
    }

    fn next_task(&mut self) -> Next {
        loop {
            let Some(queued) = self.tasks.peek() else {
                return Next::Idle;
            };
            if let Task::Timer { id, .. } = queued.task
                && !self.timers.contains_key(&id)
            {
                self.tasks.pop();
                continue;
            }
            if queued.due > self.limits.max_virtual_time_ms {
                return Next::Abort(Abort::VirtualTime);
            }
            if self.jobs_run >= self.limits.max_tasks {
                return Next::Abort(Abort::Tasks);
            }
            let Some(queued) = self.tasks.pop() else {
                return Next::Idle; // cov:ignore: peeked just above
            };
            self.now = self.now.max(queued.due);
            self.jobs_run += 1;
            return Next::Run(queued.task);
        }
    }

    fn next_microtask(&mut self) -> Result<Option<Microtask>, Abort> {
        if self.microtasks.is_empty() {
            return Ok(None);
        }
        if self.jobs_run >= self.limits.max_tasks {
            return Err(Abort::Tasks);
        }
        self.jobs_run += 1;
        Ok(self.microtasks.pop_front())
    }

    /// Discard every queue and remember why.
    fn abort(&mut self, reason: Abort) {
        self.tasks.clear();
        self.microtasks.clear();
        self.timers.clear();
        self.frame_callbacks.clear();
        self.frame_scheduled = false;
        self.aborted = Some(reason);
    }
}

/// The Boa [`JobExecutor`] of every runtime. Promise, generic, and async
/// jobs become microtasks; Boa timeout and interval jobs become tasks on
/// the virtual clock. All queues live in [`super::State`], never in the
/// executor itself, so the executor keeps no handle to shared state.
pub(crate) struct RaikiriJobExecutor;

impl JobExecutor for RaikiriJobExecutor {
    fn enqueue_job(self: Rc<Self>, job: Job, context: &mut Context) {
        // Boa enqueues jobs while script runs (promise resolution), never
        // while a binding holds the `State` borrow: bindings do not call into
        // the engine inside `with_state`. If that ever broke, `with_state`
        // would record a re-entrancy host failure and the job would be
        // dropped, rather than panicking.
        let _ = with_state(context, |state| {
            let event_loop = &mut state.event_loop;
            match job {
                Job::PromiseJob(job) => event_loop.microtasks.push_back(Microtask::Promise(job)),
                Job::GenericJob(job) => event_loop.microtasks.push_back(Microtask::Generic(job)),
                Job::AsyncJob(job) => event_loop.microtasks.push_back(Microtask::Async(job)),
                Job::TimeoutJob(job) => {
                    let delay = job.timeout().as_millis() as f64;
                    let native = NativeJob::new(move |context| {
                        if job.cancelled() {
                            return Ok(JsValue::undefined());
                        }
                        job.call(context)
                    });
                    event_loop.queue_task(delay, Task::Native(native));
                }
                Job::IntervalJob(job) => {
                    let delay = job.interval().as_millis() as f64;
                    let native = NativeJob::new(move |context| {
                        if job.cancelled() {
                            return Ok(JsValue::undefined());
                        }
                        let result = job.call(context);
                        context.enqueue_job(Job::IntervalJob(job));
                        result
                    });
                    event_loop.queue_task(delay, Task::Native(native));
                }
                // Finalization registry cleanup is optional (ECMA-262
                // `HostEnqueueFinalizationRegistryCleanupJob`); this
                // runtime never runs it. Any other future job kind is
                // dropped the same way.
                _ => {}
            }
        });
    }

    /// Only Rust callers reach this (`Context::run_jobs`); script cannot.
    /// A limit reached here is returned as a plain error for the caller, and
    /// the abort is already recorded, so later runs keep refusing.
    fn run_jobs(self: Rc<Self>, context: &mut Context) -> JsResult<()> {
        microtask_checkpoint(context).map_err(|abort| {
            JsNativeError::error()
                .with_message(abort.to_string())
                .into()
        })
    }
}

/// The runtime's abort reason, if a limit was reached earlier.
pub(crate) fn aborted(context: &mut Context) -> Option<Abort> {
    with_state(context, |state| state.event_loop.aborted.clone())
        .ok()
        .flatten()
}

/// Discard every queue and record `reason`.
pub(crate) fn abort(context: &mut Context, reason: Abort) -> Abort {
    let _ = with_state(context, |state| state.event_loop.abort(reason.clone()));
    reason
}

/// Handle an error from a callback the loop invoked: a limit aborts the
/// run, anything else is recorded as an uncaught exception.
fn settle(context: &mut Context, result: JsResult<JsValue>) -> Result<(), Abort> {
    let Err(error) = result else {
        return Ok(());
    };
    if let Some(reason) = abort_for(&error) {
        return Err(abort(context, reason));
    }
    let message = error_message(&error, context);
    let _ = with_state(context, |state| {
        state.event_loop.uncaught_errors.push(message)
    });
    Ok(())
}

/// HTML "perform a microtask checkpoint": run microtasks until the queue is
/// empty, including ones queued along the way.
pub(crate) fn microtask_checkpoint(context: &mut Context) -> Result<(), Abort> {
    loop {
        let next = with_state(context, |state| state.event_loop.next_microtask());
        let microtask = match next {
            Ok(Ok(Some(microtask))) => microtask,
            Ok(Err(reason)) => return Err(abort(context, reason)),
            _ => return Ok(()),
        };
        let result = match microtask {
            Microtask::Promise(job) => job.call(context),
            Microtask::Generic(job) => job.call(context),
            Microtask::Async(job) => poll_async_job(job, context),
            Microtask::Callback(callback) => callback.call(&JsValue::undefined(), &[], context),
        };
        settle(context, result)?;
    }
}

/// Drive an async job as far as it goes without waiting: this runtime has
/// no reactor, so a future that is still pending after one poll is dropped.
fn poll_async_job(job: NativeAsyncJob, context: &mut Context) -> JsResult<JsValue> {
    let cell = RefCell::new(context);
    let mut future = pin!(job.call(&cell));
    match future
        .as_mut()
        .poll(&mut TaskContext::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => Ok(JsValue::undefined()),
    }
}

/// Run tasks and microtasks until both queues are empty or a limit is hit.
pub(crate) fn run_until_idle(context: &mut Context) -> Result<(), Abort> {
    if let Some(reason) = aborted(context) {
        return Err(reason);
    }
    microtask_checkpoint(context)?;
    loop {
        let Ok(next) = with_state(context, |state| state.event_loop.next_task()) else {
            return Ok(()); // cov:ignore: the loop never runs inside a binding
        };
        let task = match next {
            Next::Idle => return Ok(()),
            Next::Abort(reason) => return Err(abort(context, reason)),
            Next::Run(task) => task,
        };
        match task {
            Task::Timer { id, nesting } => run_timer(context, id, nesting)?,
            Task::AnimationFrame => run_animation_frame(context)?,
            Task::Native(job) => {
                let result = job.call(context);
                settle(context, result)?;
            }
        }
        microtask_checkpoint(context)?;
    }
}

/// HTML §8.6 timer task steps: run the handler, then either repeat the
/// timer or remove it from the map of active timers.
fn run_timer(context: &mut Context, id: i32, nesting: u32) -> Result<(), Abort> {
    let handler = with_state(context, |state| {
        let event_loop = &mut state.event_loop;
        let timer = event_loop.timers.get(&id)?;
        let handler = match &timer.handler {
            Handler::Callback(callback) => Handler::Callback(callback.clone()),
            Handler::Code(code) => Handler::Code(code.clone()),
        };
        let args = timer.args.clone();
        event_loop.current_nesting = Some(nesting);
        Some((handler, args))
    });
    let Ok(Some((handler, args))) = handler else {
        return Ok(()); // cov:ignore: timers removed before their task are skipped in `next_task`
    };
    let result = match handler {
        Handler::Callback(callback) => {
            let this = JsValue::from(context.global_object());
            callback.call(&this, &args, context)
        }
        Handler::Code(code) => context.eval(Source::from_bytes(&code)),
    };
    settle(context, result)?;
    // "Clean up after running script": the checkpoint belongs to the timer
    // task, so timers its microtasks set inherit the task's nesting level.
    microtask_checkpoint(context)?;
    let _ = with_state(context, |state| {
        let event_loop = &mut state.event_loop;
        match event_loop.timers.get(&id) {
            Some(timer) if timer.repeat => {
                let timeout = timer.timeout;
                event_loop.schedule_timer(id, timeout);
            }
            _ => {
                event_loop.timers.remove(&id);
            }
        }
        event_loop.current_nesting = None;
    });
    Ok(())
}

/// Run the animation frame callbacks registered before this frame, in
/// registration order, all with the frame's timestamp. Callbacks registered
/// while the frame runs wait for the next frame; ones cancelled while it
/// runs are skipped.
fn run_animation_frame(context: &mut Context) -> Result<(), Abort> {
    let setup = with_state(context, |state| {
        let event_loop = &mut state.event_loop;
        event_loop.frame_scheduled = false;
        (event_loop.next_frame_handle, event_loop.now)
    });
    let Ok((end, timestamp)) = setup else {
        return Ok(()); // cov:ignore: the loop never runs inside a binding
    };
    loop {
        let next = with_state(context, |state| {
            let callbacks = &mut state.event_loop.frame_callbacks;
            let (&handle, _) = callbacks.first_key_value()?;
            if handle >= end {
                return None;
            }
            callbacks.remove(&handle)
        });
        let Ok(Some(callback)) = next else {
            return Ok(());
        };
        let result = callback.call(&JsValue::undefined(), &[JsValue::from(timestamp)], context);
        settle(context, result)?;
        // Each callback is its own "clean up after running script".
        microtask_checkpoint(context)?;
    }
}

fn callable_arg(args: &[JsValue], what: &str) -> JsResult<JsObject> {
    args.first().and_then(JsValue::as_callable).ok_or_else(|| {
        JsNativeError::typ()
            .with_message(format!("{what} is not a function"))
            .into()
    })
}

/// HTML §8.6 timer initialization steps for a new timer.
fn set_timer(args: &[JsValue], repeat: bool, context: &mut Context) -> JsResult<JsValue> {
    // `handler` is a required WebIDL argument.
    let Some(handler_arg) = args.first().cloned() else {
        return Err(JsNativeError::typ()
            .with_message("a timer handler is required")
            .into());
    };
    let handler = match handler_arg.as_callable() {
        Some(callback) => Handler::Callback(callback.clone()),
        None => Handler::Code(handler_arg.to_string(context)?.to_std_string_escaped()),
    };
    let timeout = args.get(1).cloned().unwrap_or_default().to_i32(context)?;
    let rest = args.get(2..).unwrap_or_default().to_vec();
    with_state(context, |state| {
        let event_loop = &mut state.event_loop;
        let id = event_loop.next_timer_id;
        event_loop.next_timer_id = id.saturating_add(1);
        event_loop.timers.insert(
            id,
            Timer {
                handler,
                args: rest,
                timeout,
                repeat,
            },
        );
        event_loop.schedule_timer(id, timeout);
        JsValue::from(id)
    })
}

fn set_timeout(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    set_timer(args, false, context)
}

fn set_interval(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    set_timer(args, true, context)
}

/// `clearTimeout`/`clearInterval`: both remove from the one map of active
/// timers.
fn clear_timer(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = args.first().cloned().unwrap_or_default().to_i32(context)?;
    with_state(context, |state| {
        state.event_loop.timers.remove(&id);
    })?;
    Ok(JsValue::undefined())
}

/// HTML `requestAnimationFrame`: frames fall on 16ms boundaries of
/// the virtual clock.
fn request_animation_frame(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let callback = callable_arg(args, "requestAnimationFrame callback")?;
    with_state(context, |state| {
        let event_loop = &mut state.event_loop;
        let handle = event_loop.next_frame_handle;
        event_loop.next_frame_handle = handle.saturating_add(1);
        event_loop.frame_callbacks.insert(handle, callback);
        if !event_loop.frame_scheduled {
            event_loop.frame_scheduled = true;
            let next_frame = ((event_loop.now / 16.0).floor() + 1.0) * 16.0;
            let delay = next_frame - event_loop.now;
            event_loop.queue_task(delay, Task::AnimationFrame);
        }
        JsValue::from(handle)
    })
}

fn cancel_animation_frame(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let handle = args.first().cloned().unwrap_or_default().to_u32(context)?;
    with_state(context, |state| {
        state.event_loop.frame_callbacks.remove(&handle);
    })?;
    Ok(JsValue::undefined())
}

/// HTML `queueMicrotask`.
fn queue_microtask(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let callback = callable_arg(args, "queueMicrotask callback")?;
    with_state(context, |state| {
        state
            .event_loop
            .microtasks
            .push_back(Microtask::Callback(callback));
    })?;
    Ok(JsValue::undefined())
}

/// `performance.now()` (High Resolution Time): the virtual clock.
fn performance_now(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    with_state(context, |state| JsValue::from(state.event_loop.now))
}

/// Define the timer, animation frame, and microtask operations and the
/// `performance` object on the global object.
pub(crate) fn install(context: &mut Context) -> JsResult<()> {
    let operations: [(
        &str,
        usize,
        boa_engine::native_function::NativeFunctionPointer,
    ); 7] = [
        ("setTimeout", 1, set_timeout),
        ("setInterval", 1, set_interval),
        ("clearTimeout", 0, clear_timer),
        ("clearInterval", 0, clear_timer),
        ("requestAnimationFrame", 1, request_animation_frame),
        ("cancelAnimationFrame", 1, cancel_animation_frame),
        ("queueMicrotask", 1, queue_microtask),
    ];
    let global = context.global_object();
    for (name, length, f) in operations {
        let operation = function(context, name, length, f)?;
        define_operation(&global, name, operation.into(), context)?;
    }
    let performance = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(performance_now),
            JsString::from("now"),
            0,
        )
        .build();
    define_operation(&global, "performance", performance.into(), context)
}

fn define_operation(
    global: &JsObject,
    name: &str,
    value: JsValue,
    context: &mut Context,
) -> JsResult<()> {
    let descriptor = PropertyDescriptor::builder()
        .value(value)
        .writable(true)
        .enumerable(true)
        .configurable(true);
    global.define_property_or_throw(JsString::from(name), descriptor, context)?;
    Ok(())
}

#[cfg(test)]
mod tests;
