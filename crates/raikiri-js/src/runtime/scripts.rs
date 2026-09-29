//! `DomRuntime::run_document`: HTML's script processing model
//! (<https://html.spec.whatwg.org/multipage/scripting.html>), run as a
//! single batch after the whole document has already been parsed (a
//! streaming parser's own, mid-parse use of the same seam is out of scope
//! here -- see [`raikiri_traits::ScriptExecutor`]'s own doc comment).
//!
//! Collects every HTML-namespace `<script>` element in tree order, runs each
//! classic one (inline, or fetched through [`super::host::DocumentHost::
//! fetch_script`]), and drives `document.readyState` through
//! `Loading`/`Interactive`/`Complete`, firing `DOMContentLoaded` and `load`
//! along the way and draining the event loop between them.
//!
//! `async` and `defer` are not distinguished: every classic script runs in
//! tree order regardless of either attribute, an approximation of their real
//! scheduling rather than a full implementation of it. A `<script>` element
//! inserted into the document while a run is already in progress (by an
//! earlier script in the same run) is not picked up by that run: the script
//! list is collected once, up front, not re-scanned as the document changes.

use boa_engine::{Context, Source};
use raikiri_dom::NodeKind;
use raikiri_traits::dom::NodeId;
use raikiri_traits::script::{ScriptExecution, ScriptExecutor};

use super::document::{self, ReadyState};
use super::event_loop::{self, Abort};
use super::host::HostError;
use super::interfaces::HTML_NS;
use super::webidl::{take_host_failure, with_state};
use super::{DomRuntime, dispatch};

/// One [`DomRuntime::run_document`] call's outcome.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RunReport {
    /// How many classic scripts were actually evaluated (an external script
    /// whose fetch failed is not counted; one whose evaluation threw an
    /// uncaught exception is).
    pub scripts_run: usize,
    /// Uncaught script exceptions, in the order they happened: each one is
    /// also reported through `window`'s `error` event first (HTML "report an
    /// exception"), and only ends up here if nothing cancelled that event.
    pub uncaught_errors: Vec<String>,
    /// External scripts whose `src` could not be resolved or fetched,
    /// `"url: reason"`, in the order they happened. Each one also fires a
    /// trusted `error` event at its `<script>` element; the script itself is
    /// never evaluated.
    pub fetch_errors: Vec<String>,
    /// The resource limit that stopped the run, if any -- the same
    /// [`Abort`] a direct [`DomRuntime::run_until_idle`] call would return.
    /// Once set, the document is only partially run: everything up to the
    /// point of the abort already happened and is visible through the host,
    /// but nothing after it did.
    pub aborted: Option<Abort>,
    /// Embedder (host) failures recorded during the run -- layout, a
    /// stylesheet, or fragment parsing failing underneath a script's own DOM
    /// mutation -- in the order they happened. Distinct from a resource
    /// limit: a host failure does not stop the run. A host failure also
    /// surfaces to script as an ordinary thrown `Error` (this runtime's
    /// existing convention throughout, not specific to script running: the
    /// same failure a binding records here is also what it throws), so an
    /// uncaught one can appear in both this field and [`Self::
    /// uncaught_errors`] at once.
    pub host_failures: Vec<String>,
    /// `console.*` calls made during the run, `(level, message)`, in call
    /// order (`level` is one of `"log"`/`"error"`/`"warn"`/`"info"`/
    /// `"debug"`).
    pub console: Vec<(String, String)>,
}

/// The JavaScript MIME type essence strings a `<script>`'s trimmed `type`
/// attribute must be an ASCII case-insensitive match for -- as the whole
/// string, exactly as given -- to be "a JavaScript MIME type essence match"
/// and so run as a classic script. This is a match against these fixed
/// essence strings themselves, not an operation that first reduces the
/// attribute value to its own essence by stripping any `;`-delimited
/// parameters: `text/javascript; charset=utf-8` is therefore not a match (it
/// is not equal, as a whole string, to any entry below), unlike plain
/// `text/javascript`. Anything that is not a match -- `module`, `text/
/// plain`, `text/javascript; charset=utf-8` -- is not executed. A missing or
/// empty `type` attribute is also a classic script (checked separately in
/// [`is_classic_script_type`], not part of this list).
const JAVASCRIPT_MIME_ESSENCES: &[&str] = &[
    "application/ecmascript",
    "application/javascript",
    "application/x-ecmascript",
    "application/x-javascript",
    "text/ecmascript",
    "text/javascript",
    "text/javascript1.0",
    "text/javascript1.1",
    "text/javascript1.2",
    "text/javascript1.3",
    "text/javascript1.4",
    "text/javascript1.5",
    "text/jscript",
    "text/livescript",
    "text/x-ecmascript",
    "text/x-javascript",
];

/// Whether a `<script type="...">` value (already read from the attribute,
/// `None` when the attribute is absent) marks the element as a classic
/// script: absent, empty (after trimming ASCII whitespace), or a JavaScript
/// MIME type essence match (see [`JAVASCRIPT_MIME_ESSENCES`]'s own doc
/// comment for exactly what that requires). `nomodule` and `language` are
/// both ignored by every caller of this function -- this runtime has no
/// module support, so a `nomodule` script still runs as classic, and
/// `language` has been obsolete since HTML4.
fn is_classic_script_type(type_value: Option<&str>) -> bool {
    let trimmed = type_value.unwrap_or("").trim();
    if trimmed.is_empty() {
        return true;
    }
    JAVASCRIPT_MIME_ESSENCES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(trimmed))
}

/// A minimal absolute/relative merge for a script's `src`, narrow enough for
/// this runtime's own needs rather than a general URL parser (the same
/// scope tradeoff as `window::LocationParts::parse`, and for the same
/// reason: neither `url` nor any other new dependency is available to this
/// crate to do it properly). Four `src` shapes: a scheme-absolute one
/// (containing `://`) is used as-is; a scheme-relative one (`//host/path`)
/// reuses `base`'s scheme; an absolute-path one (`/path`) reuses `base`'s
/// scheme and authority, replacing its path outright; anything else is
/// merged with `base`'s own directory (everything up to and including its
/// last `/`, ignoring any query or fragment, or `/` itself when its path has
/// none). Returns `None` when `src` is relative (in any of the last three
/// senses) and there is no `base` to resolve it against, or `base` is not of
/// the `scheme://authority[/path]` shape this function understands -- both
/// treated by the caller as a fetch failure, the same as a resolvable URL
/// that simply fails to fetch.
fn resolve_script_url(base: Option<&str>, src: &str) -> Option<String> {
    if src.contains("://") {
        return Some(src.to_owned());
    }
    let base = base?;
    let scheme_end = base.find("://")?;
    let scheme = &base[..scheme_end];
    let authority_start = scheme_end + 3;
    let rest = &base[authority_start..];
    let path_start = authority_start + rest.find('/').unwrap_or(rest.len());
    if src.starts_with("//") {
        return Some(format!("{scheme}:{src}"));
    }
    if src.starts_with('/') {
        let authority = &base[authority_start..path_start];
        return Some(format!("{scheme}://{authority}{src}"));
    }
    let prefix = &base[..path_start];
    let path = base[path_start..].split(['?', '#']).next().unwrap_or("");
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    Some(format!("{prefix}{dir}{src}"))
}

/// Every HTML-namespace `<script>` element in `doc`, in tree order, found
/// with an explicit stack (a script can build an arbitrarily deep tree).
/// Every element found here is a candidate: [`Executor::execute_script`]
/// still decides, per element, whether its `type` marks it as a classic
/// script worth running.
fn scripts_in_tree_order(doc: &raikiri_dom::Document) -> Option<Vec<usize>> {
    let mut scripts = Vec::new();
    let mut stack: Vec<usize> = doc
        .get_node(doc.root_index())?
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    while let Some(index) = stack.pop() {
        let Some(node) = doc.get_node(index) else {
            continue; // cov:ignore: every stack entry comes from a resolved node's own children, always valid in the same arena
        };
        if node.kind() == NodeKind::Element
            && node.tag_name() == Some("script")
            && doc.element_namespace_uri(index) == Some(HTML_NS)
        {
            scripts.push(index);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    Some(scripts)
}

fn collect_script_elements(context: &mut Context) -> Vec<usize> {
    with_state(context, |s| scripts_in_tree_order(s.host.document()))
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Take any host failure recorded since the last drain and record it in
/// `report.host_failures`. Called after every stage (each script, each
/// lifecycle event, each event-loop drain) so that one stage's host failure
/// is never silently folded into, or lost behind, another's.
fn drain_host_failure_into(context: &mut Context, report: &mut RunReport) {
    if let Some(message) = take_host_failure(context) {
        report.host_failures.push(message);
    }
}

/// Run tasks and microtasks until both queues are empty or a limit is hit
/// (shared by the two points in [`run`]'s flow that drain the event loop:
/// once after `DOMContentLoaded`, once after `load`), recording an abort and
/// draining any host failure the callbacks it ran may have caused.
fn drain_event_loop(context: &mut Context, report: &mut RunReport) {
    if let Err(reason) = event_loop::run_until_idle(context) {
        report.aborted = Some(reason);
    }
    drain_host_failure_into(context, report);
}

/// Evaluate one script's source: run it, then HTML's "report an exception"
/// for an uncaught error, then a microtask checkpoint -- the shared tail of
/// "run the classic script" and "run the script" that follows every
/// classic-script evaluation, whether inline or external. `source` is the
/// script's own location (its resolved `src`, or the document's URL for an
/// inline script), threaded through to the reported `ErrorEvent.filename`
/// when known. `Err` means a resource limit was reached either running the
/// script or reporting its exception; the caller stops the whole run.
fn evaluate_script(context: &mut Context, code: &str, source: Option<&str>) -> Result<(), Abort> {
    if let Some(reason) = event_loop::aborted(context) {
        // cov:ignore: every current caller (`Executor::run_one`, reached
        // only through `ScriptExecutor::execute_script`, which checks this
        // same condition itself first) already refuses to call this
        // function once aborted; kept so this function stays correct on
        // its own if a future caller does not.
        return Err(reason);
    }
    let result = context.eval(Source::from_bytes(code));
    if let Err(error) = &result {
        if let Some(reason) = event_loop::abort_for(error) {
            return Err(event_loop::abort(context, reason));
        }
        if let Err(report_error) = dispatch::report_exception(context, error, source) {
            let reason = event_loop::abort_for(&report_error).unwrap_or(Abort::Recursion);
            return Err(event_loop::abort(context, reason));
        }
    }
    event_loop::microtask_checkpoint(context)
}

/// Fire a trusted event and record an abort if dispatching it hit a
/// resource limit, draining any host failure the listeners it ran may have
/// caused either way. Shared by a script's own `load`/`error` and the
/// document's `DOMContentLoaded`/`load`.
fn fire_lifecycle_event(
    context: &mut Context,
    report: &mut RunReport,
    target: Option<usize>,
    kind: &str,
    bubbles: bool,
) {
    if event_loop::aborted(context).is_some() {
        // cov:ignore: `run`'s own `report.aborted` is seeded from this same
        // `event_loop::aborted` read at entry (see its own comment), and
        // every call site here is gated on `report.aborted.is_none()` (or
        // reached only after `evaluate_script` returned `Ok`, which the
        // same guard already implies transitively); so this can no longer
        // be true at any current call site. Kept so this function stays
        // correct on its own if a future call site does not check first.
        return;
    }
    if let Err(error) = dispatch::fire_event(context, target, kind, bubbles, false) {
        let reason = event_loop::abort_for(&error).unwrap_or(Abort::Recursion);
        report.aborted = Some(event_loop::abort(context, reason));
    }
    drain_host_failure_into(context, report);
}

/// Runs one script element at a time, in the context of one [`RunReport`]
/// under construction. Implements the [`ScriptExecutor`] boundary: a future
/// streaming parser would hold one of these across a whole parse instead of
/// just the loop in [`run`].
struct Executor<'a> {
    context: &'a mut Context,
    report: &'a mut RunReport,
}

impl<'a> Executor<'a> {
    fn new(context: &'a mut Context, report: &'a mut RunReport) -> Self {
        Self { context, report }
    }

    /// `index` is already known to be a classic `<script>` element
    /// ([`ScriptExecutor::execute_script`] checked it); this runs it end to
    /// end: `currentScript`, inline vs. external, the evaluation itself, and
    /// the script's own `load`/`error` event.
    ///
    /// `currentScript` is set only around the evaluation itself (HTML
    /// "execute the script element" sets it back to its old value
    /// immediately after running the script, *before* the load event that
    /// follows a successful external fetch): [`Self::run_inline`] and
    /// [`Self::run_external`] each set and clear it themselves, tightly
    /// around their own [`evaluate_script`] call, rather than this function
    /// doing it once around the whole dispatch -- a script that never
    /// reaches evaluation at all (an empty `src`, an unresolvable one, or a
    /// fetch failure) never has `currentScript` set for it in the first
    /// place, matching that a fetch failure never reaches "execute" either.
    fn run_one(&mut self, index: usize) {
        let base_url = with_state(self.context, |s| s.host.document_url())
            .ok()
            .flatten();
        let src = with_state(self.context, |s| {
            s.host
                .document()
                .element_attribute(index, "src")
                .map(str::to_owned)
        })
        .ok()
        .flatten();

        match src {
            Some(src) => self.run_external(index, &src, base_url.as_deref()),
            None => self.run_inline(index, base_url.as_deref()),
        }
        drain_host_failure_into(self.context, self.report);
    }

    fn run_inline(&mut self, index: usize, base_url: Option<&str>) {
        let code = with_state(self.context, |s| {
            document::child_text_content(s.host.document(), index)
        })
        .unwrap_or_default();
        self.report.scripts_run += 1;
        let _ = with_state(self.context, |s| s.current_script = Some(index));
        let result = evaluate_script(self.context, &code, base_url);
        let _ = with_state(self.context, |s| s.current_script = None);
        if let Err(reason) = result {
            self.report.aborted = Some(reason);
        }
    }

    fn run_external(&mut self, index: usize, src: &str, base_url: Option<&str>) {
        if src.is_empty() {
            self.fetch_failed(index, "<empty src>", "the src attribute is empty");
            return;
        }
        let Some(url) = resolve_script_url(base_url, src) else {
            self.fetch_failed(index, src, "the script URL could not be resolved");
            return;
        };
        let Ok(fetched) = with_state(self.context, |s| s.host.fetch_script(&url)) else {
            return; // cov:ignore: no JS callback runs before this, so the state cannot already be borrowed
        };
        match fetched {
            Ok(code) => {
                self.report.scripts_run += 1;
                let _ = with_state(self.context, |s| s.current_script = Some(index));
                let result = evaluate_script(self.context, &code, Some(&url));
                let _ = with_state(self.context, |s| s.current_script = None);
                if let Err(reason) = result {
                    self.report.aborted = Some(reason);
                    return;
                }
                fire_lifecycle_event(self.context, self.report, Some(index), "load", false);
            }
            Err(HostError(message)) => self.fetch_failed(index, &url, &message),
        }
    }

    fn fetch_failed(&mut self, index: usize, url: &str, reason: &str) {
        self.report.fetch_errors.push(format!("{url}: {reason}"));
        fire_lifecycle_event(self.context, self.report, Some(index), "error", false);
    }
}

impl ScriptExecutor for Executor<'_> {
    fn execute_script(&mut self, element: NodeId) -> ScriptExecution {
        if event_loop::aborted(self.context).is_some() {
            return ScriptExecution::Continue;
        }
        let Ok(index) = usize::try_from(element.0) else {
            // cov:ignore: `usize` and `u64` are the same width on every
            // target this crate builds for today, so this conversion never
            // fails; kept so the code stays correct if that ever changes.
            return ScriptExecution::Continue;
        };
        let is_classic = with_state(self.context, |s| {
            let doc = s.host.document();
            let Some(node) = doc.get_node(index) else {
                return false;
            };
            node.kind() == NodeKind::Element
                && node.tag_name() == Some("script")
                && doc.element_namespace_uri(index) == Some(HTML_NS)
                && is_classic_script_type(doc.element_attribute(index, "type"))
        })
        .unwrap_or(false);
        if is_classic {
            self.run_one(index);
        }
        ScriptExecution::Continue
    }
}

/// [`DomRuntime::run_document`]'s actual work (idempotency is that method's
/// own concern, via its cached [`RunReport`]): mark the document's flags,
/// collect and run every classic script in tree order, then drive
/// `readyState` through `Interactive`/`Complete`, firing `DOMContentLoaded`
/// and `load` and draining the event loop in between, per HTML's script
/// processing model (<https://html.spec.whatwg.org/multipage/scripting.html>).
pub(crate) fn run(runtime: &mut DomRuntime) -> RunReport {
    let mut report = RunReport::default();
    let context = runtime.context_mut();

    // Seed from the runtime's own abort state before anything else: a
    // runtime that was already aborted (by an earlier, unrelated `evaluate`
    // call) before `run_document` was ever called must not read back as
    // `aborted: None` just because the script loop below happens to find no
    // (or no *reachable*) script of its own to blame it on.
    report.aborted = event_loop::aborted(context);

    let _ = with_state(context, |s| {
        s.host.document_mut().mark_in_document_flags();
        s.ready_state = ReadyState::Loading;
    });
    let uncaught_before = with_state(context, |s| s.event_loop.uncaught_errors.len()).unwrap_or(0);
    let console_before = with_state(context, |s| s.console.len()).unwrap_or(0);

    if report.aborted.is_none()
        && let Err(error) = super::dispatch::sync_all_event_handler_content_attributes(context)
    {
        let reason = event_loop::abort_for(&error).unwrap_or(Abort::Recursion);
        report.aborted = Some(event_loop::abort(context, reason));
    }

    let elements = collect_script_elements(context);
    {
        let mut executor = Executor::new(context, &mut report);
        for index in elements {
            if event_loop::aborted(executor.context).is_some() {
                break;
            }
            executor.execute_script(NodeId::new(index as u64));
        }
    }
    drain_host_failure_into(context, &mut report);

    if report.aborted.is_none() {
        let _ = with_state(context, |s| s.ready_state = ReadyState::Interactive);
        let document_index = with_state(context, |s| s.host.document().root_index()).unwrap_or(0);
        fire_lifecycle_event(
            context,
            &mut report,
            Some(document_index),
            "DOMContentLoaded",
            true,
        );
    }

    if report.aborted.is_none() {
        drain_event_loop(context, &mut report);
    }

    if report.aborted.is_none() {
        let _ = with_state(context, |s| s.ready_state = ReadyState::Complete);
        fire_lifecycle_event(context, &mut report, None, "load", false);
        if report.aborted.is_none() {
            drain_event_loop(context, &mut report);
        }
    }

    report.uncaught_errors = with_state(context, |s| {
        s.event_loop.uncaught_errors[uncaught_before..].to_vec()
    })
    .unwrap_or_default();
    report.console = with_state(context, |s| {
        s.console[console_before..]
            .iter()
            .map(|m| (m.level.as_str().to_owned(), m.message.clone()))
            .collect()
    })
    .unwrap_or_default();

    report
}

#[cfg(test)]
mod tests;
