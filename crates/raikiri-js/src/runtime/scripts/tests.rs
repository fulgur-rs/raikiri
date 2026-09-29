use std::any::Any;

use boa_engine::JsString;
use boa_engine::property::Attribute;
use raikiri_dom::Document;
use raikiri_traits::dom::NodeId;
use raikiri_traits::script::{ScriptExecution, ScriptExecutor};

use super::{Executor, RunReport, is_classic_script_type, resolve_script_url};
use crate::runtime::host::{BoxGeometry, DocumentHost, HostError};
use crate::runtime::interfaces::wrap;
use crate::runtime::test_host::StubHost;
use crate::runtime::webidl::with_state;
use crate::runtime::{Abort, DomRuntime, Limits, RunOptions};

fn runtime_with(limits: Limits, host: StubHost) -> DomRuntime {
    DomRuntime::with_options(host, RunOptions { limits }).unwrap()
}

/// Create a `<script>` element with the given attributes and inline text
/// (skipped when empty), append it to `parent`, and return its index.
fn append_script(doc: &mut Document, parent: usize, attrs: &[(&str, &str)], text: &str) -> usize {
    let script = doc.create_detached_element("script").unwrap();
    for &(name, value) in attrs {
        doc.set_element_attribute(script, name, value).unwrap();
    }
    if !text.is_empty() {
        doc.append_text(script, text);
    }
    doc.append_child(parent, script).unwrap();
    script
}

/// Expose a wrapped node as a global, so a script can compare
/// `document.currentScript` against it by identity.
fn expose_node(rt: &mut DomRuntime, name: &str, index: usize) {
    let object = wrap(rt.context_mut(), index).unwrap();
    rt.context_mut()
        .register_global_property(JsString::from(name), object, Attribute::all())
        .unwrap();
}

fn eval_string(rt: &mut DomRuntime, src: &str) -> String {
    let value = rt.evaluate(src).unwrap();
    value
        .to_string(rt.context_mut())
        .unwrap()
        .to_std_string_escaped()
}

fn eval_bool(rt: &mut DomRuntime, src: &str) -> bool {
    rt.evaluate(src).unwrap().to_boolean()
}

/// Read a null-namespace attribute straight off the arena via `with_state`,
/// bypassing `DomRuntime::evaluate`'s abort gate -- for use after a test
/// deliberately aborted the runtime, when `evaluate` itself would just
/// return `RuntimeError::Aborted` instead of answering the question.
fn attribute(rt: &mut DomRuntime, index: usize, name: &str) -> Option<String> {
    with_state(rt.context_mut(), |s| {
        s.host
            .document()
            .element_attribute(index, name)
            .map(str::to_owned)
    })
    .unwrap()
}

/// A host that leaves `fetch_script` at [`DocumentHost`]'s own default
/// (`Err`, "script fetching is not supported"): [`StubHost`] always
/// overrides it, so nothing else in this file exercises the default trait
/// body.
struct DefaultFetchHost(StubHost);

impl DocumentHost for DefaultFetchHost {
    fn document(&self) -> &Document {
        self.0.document()
    }
    fn document_mut(&mut self) -> &mut Document {
        self.0.document_mut()
    }
    fn flush(&mut self) -> Result<(), HostError> {
        self.0.flush()
    }
    fn box_geometry(&mut self, node: usize) -> Result<Option<BoxGeometry>, HostError> {
        self.0.box_geometry(node)
    }
    fn computed_value(&mut self, node: usize, property: &str) -> Result<Option<String>, HostError> {
        self.0.computed_value(node, property)
    }
    fn parse_fragment(
        &mut self,
        context_tag: &str,
        context_ns: &str,
        markup: &str,
    ) -> Result<Document, HostError> {
        self.0.parse_fragment(context_tag, context_ns, markup)
    }
    fn document_url(&self) -> Option<String> {
        self.0.document_url()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

// ---- run_document: order, currentScript, readyState, lifecycle events ----

#[test]
fn classic_scripts_run_in_document_order_with_currentscript_and_lifecycle() {
    let (mut host, _, _, body) = StubHost::page();
    host.document_url = Some("https://example.test/page.html".to_owned());
    host.scripts.insert(
        "https://example.test/ext.js".to_owned(),
        "log.push('s3:' + (document.currentScript === s3) + ':' + document.readyState);".to_owned(),
    );
    let s1 = append_script(
        &mut host.document,
        body,
        &[],
        "var log = [];\
         log.push('s1:' + (document.currentScript === s1) + ':' + document.readyState);\
         document.addEventListener('DOMContentLoaded', function () { \
             log.push('dcl:' + document.readyState); \
         });\
         window.addEventListener('load', function () { \
             log.push('load:' + document.readyState); \
         });\
         s3.addEventListener('load', function () { \
             log.push('s3-load-currentscript-null:' + (document.currentScript === null)); \
         });",
    );
    let s2 = append_script(
        &mut host.document,
        body,
        &[],
        "log.push('s2:' + (document.currentScript === s2) + ':' + document.readyState);",
    );
    let s3 = append_script(&mut host.document, body, &[("src", "ext.js")], "");

    let mut rt = DomRuntime::new(host).unwrap();
    expose_node(&mut rt, "s1", s1);
    expose_node(&mut rt, "s2", s2);
    expose_node(&mut rt, "s3", s3);

    let report = rt.run_document();

    assert_eq!(report.scripts_run, 3, "{report:?}");
    assert!(report.uncaught_errors.is_empty(), "{report:?}");
    assert!(report.fetch_errors.is_empty(), "{report:?}");
    assert_eq!(report.aborted, None);

    let log = eval_string(&mut rt, "log.join('|')");
    assert_eq!(
        log,
        "s1:true:loading|s2:true:loading|s3:true:loading|\
         s3-load-currentscript-null:true|dcl:interactive|load:complete"
    );
}

#[test]
fn type_attribute_gates_classic_script_execution() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "var ran = []; ran.push('no-type');",
    );
    append_script(
        &mut host.document,
        body,
        &[("type", "module")],
        "ran.push('module');",
    );
    append_script(
        &mut host.document,
        body,
        &[("type", "text/plain")],
        "ran.push('text-plain');",
    );
    append_script(
        &mut host.document,
        body,
        &[("type", "text/javascript;charset=utf-8")],
        "ran.push('classic-with-params');",
    );

    let mut rt = DomRuntime::new(host).unwrap();
    let report = rt.run_document();

    // HTML's "JavaScript MIME type essence match" is a whole-string, ASCII
    // case-insensitive match against the classic-script list, not an
    // operation that strips `;`-delimited parameters from the attribute
    // value first: `text/javascript; charset=utf-8` is therefore not
    // recognized and does not run, exactly like `module`/`text/plain`.
    assert_eq!(report.scripts_run, 1, "{report:?}");
    assert_eq!(eval_string(&mut rt, "ran.join(',')"), "no-type");
}

#[test]
fn external_script_fetch_failures_are_recorded_and_do_not_stop_the_run() {
    let (mut host, _, _, body) = StubHost::page();
    host.document_url = Some("https://example.test/page.html".to_owned());
    let missing = append_script(&mut host.document, body, &[("src", "missing.js")], "");
    let empty = append_script(&mut host.document, body, &[("src", "")], "");
    append_script(&mut host.document, body, &[], "events.push('after');");

    let mut rt = DomRuntime::new(host).unwrap();
    expose_node(&mut rt, "missing", missing);
    expose_node(&mut rt, "empty_src", empty);
    rt.evaluate(
        "var events = []; \
         missing.addEventListener('error', function (e) { \
             events.push('missing-error:' + e.bubbles + ':' + e.cancelable + ':' + \
                 e.isTrusted + ':' + (document.currentScript === null)); \
         }); \
         empty_src.addEventListener('error', function (e) { \
             events.push('empty-error:' + (document.currentScript === null)); \
         });",
    )
    .unwrap();

    let report = rt.run_document();

    assert_eq!(report.scripts_run, 1, "{report:?}");
    assert_eq!(
        report.fetch_errors,
        vec![
            "https://example.test/missing.js: no script registered for \
             https://example.test/missing.js"
                .to_owned(),
            "<empty src>: the src attribute is empty".to_owned(),
        ],
        "{report:?}"
    );
    assert_eq!(
        eval_string(&mut rt, "events.join('|')"),
        "missing-error:false:false:true:true|empty-error:true|after"
    );
}

#[test]
fn external_script_without_a_document_url_cannot_resolve_a_relative_src() {
    let (mut host, _, _, body) = StubHost::page();
    // No `document_url`: a relative `src` has no base to resolve against.
    append_script(&mut host.document, body, &[("src", "relative.js")], "");

    let mut rt = DomRuntime::new(host).unwrap();
    let report = rt.run_document();

    assert_eq!(report.scripts_run, 0, "{report:?}");
    assert_eq!(
        report.fetch_errors,
        vec!["relative.js: the script URL could not be resolved".to_owned()]
    );
}

#[test]
fn external_script_source_that_infinite_loops_aborts_the_run() {
    let (mut host, _, _, body) = StubHost::page();
    host.document_url = Some("https://example.test/page.html".to_owned());
    host.scripts.insert(
        "https://example.test/loop.js".to_owned(),
        "while (true) {}".to_owned(),
    );
    append_script(&mut host.document, body, &[("src", "loop.js")], "");

    let mut rt = runtime_with(
        Limits {
            max_loop_iterations: 10_000,
            ..Default::default()
        },
        host,
    );
    let report = rt.run_document();

    assert_eq!(report.aborted, Some(Abort::LoopIterations), "{report:?}");
}

#[test]
fn fetch_script_default_trait_body_fails_and_is_recorded_as_a_fetch_error() {
    let (mut host, _, _, body) = StubHost::page();
    host.document_url = Some("https://example.test/page.html".to_owned());
    append_script(&mut host.document, body, &[("src", "missing.js")], "");

    let mut rt = DomRuntime::new(DefaultFetchHost(host)).unwrap();
    let report = rt.run_document();

    assert_eq!(report.scripts_run, 0, "{report:?}");
    assert_eq!(
        report.fetch_errors,
        vec!["https://example.test/missing.js: script fetching is not supported".to_owned()],
        "{report:?}"
    );
}

#[test]
fn window_onerror_that_itself_aborts_is_recorded_as_an_abort() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "window.onerror = function () { while (true) {} }; throw new Error('boom');",
    );

    let mut rt = runtime_with(
        Limits {
            max_loop_iterations: 10_000,
            ..Default::default()
        },
        host,
    );
    let report = rt.run_document();

    assert_eq!(report.aborted, Some(Abort::LoopIterations), "{report:?}");
}

#[test]
fn uncaught_exception_is_reported_and_later_scripts_still_run() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(&mut host.document, body, &[], "throw new Error('boom');");
    append_script(&mut host.document, body, &[], "var ran = true;");

    let mut rt = DomRuntime::new(host).unwrap();
    let report = rt.run_document();

    assert_eq!(report.scripts_run, 2, "{report:?}");
    assert_eq!(report.uncaught_errors.len(), 1, "{report:?}");
    assert!(report.uncaught_errors[0].contains("boom"), "{report:?}");
    assert!(eval_bool(&mut rt, "ran === true"));
}

#[test]
fn set_timeout_side_effects_are_visible_after_run_document() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "setTimeout(function () { document.title = 'changed'; }, 0);",
    );

    let mut rt = DomRuntime::new(host).unwrap();
    let report = rt.run_document();

    assert_eq!(report.aborted, None, "{report:?}");
    assert!(eval_bool(&mut rt, "document.title === 'changed'"));
}

#[test]
fn document_title_can_be_cleared_after_being_set() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "document.title = 'x'; document.title = '';",
    );

    let mut rt = DomRuntime::new(host).unwrap();
    let report = rt.run_document();

    assert_eq!(report.aborted, None, "{report:?}");
    assert!(eval_bool(&mut rt, "document.title === ''"));
}

#[test]
fn console_methods_are_collected_with_their_level() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "console.log('hello', 42); console.error('e'); console.warn('w'); \
         console.info('i'); console.debug('d');",
    );

    let mut rt = DomRuntime::new(host).unwrap();
    let report = rt.run_document();

    assert_eq!(
        report.console,
        vec![
            ("log".to_owned(), "hello 42".to_owned()),
            ("error".to_owned(), "e".to_owned()),
            ("warn".to_owned(), "w".to_owned()),
            ("info".to_owned(), "i".to_owned()),
            ("debug".to_owned(), "d".to_owned()),
        ]
    );
}

#[test]
fn console_output_from_before_run_document_is_excluded() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(&mut host.document, body, &[], "console.log('during');");

    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("console.log('pre')").unwrap();
    let report = rt.run_document();

    assert_eq!(
        report.console,
        vec![("log".to_owned(), "during".to_owned())]
    );
}

#[test]
fn run_document_is_idempotent() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "var counter = 0; counter += 1;",
    );

    let mut rt = DomRuntime::new(host).unwrap();
    let first = rt.run_document();
    let second = rt.run_document();

    assert_eq!(first, second);
    assert_eq!(first.scripts_run, 1);
    assert!(eval_bool(&mut rt, "counter === 1"));
}

// ---- resource limits -------------------------------------------------------

#[test]
fn run_document_on_an_already_aborted_runtime_reports_the_existing_abort() {
    let (mut host, _, _, body) = StubHost::page();
    // Never reached: the runtime is aborted (by the `evaluate` call below)
    // before `run_document` ever looks at the document's own scripts.
    append_script(&mut host.document, body, &[], "ran = true;");

    let mut rt = runtime_with(
        Limits {
            max_loop_iterations: 10_000,
            ..Default::default()
        },
        host,
    );
    let _ = rt.evaluate("while (true) {}");
    assert!(
        rt.evaluate("1").is_err(),
        "runtime should already be aborted"
    );

    let report = rt.run_document();

    assert_eq!(report.aborted, Some(Abort::LoopIterations), "{report:?}");
    assert_eq!(report.scripts_run, 0, "{report:?}");
}

#[test]
fn infinite_loop_script_aborts_and_keeps_earlier_dom_changes() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "document.body.setAttribute('data-marker', 'before'); while (true) {}",
    );
    append_script(
        &mut host.document,
        body,
        &[],
        "document.body.setAttribute('data-marker', 'after');",
    );

    let mut rt = runtime_with(
        Limits {
            max_loop_iterations: 10_000,
            ..Default::default()
        },
        host,
    );
    let report = rt.run_document();

    assert_eq!(report.aborted, Some(Abort::LoopIterations));
    assert_eq!(
        attribute(&mut rt, body, "data-marker"),
        Some("before".to_owned())
    );
}

#[test]
fn node_budget_limit_aborts_element_creation_and_keeps_earlier_ones() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "for (;;) { document.createElement('i'); }",
    );
    let before = host.document.node_count();

    let mut rt = runtime_with(
        Limits {
            max_nodes: before + 2,
            ..Default::default()
        },
        host,
    );
    let report = rt.run_document();

    assert_eq!(report.aborted, Some(Abort::Nodes));
    let node_count = with_state(rt.context_mut(), |s| s.host.document().node_count()).unwrap();
    assert_eq!(node_count, before + 2);
}

/// Run `loop_body` (a script whose whole body is expected to loop forever,
/// each iteration creating at least one node) with `max_nodes` set 2 past
/// the page's own starting node count, and assert it aborts on the node
/// budget rather than running forever.
fn assert_node_budget_aborts(loop_body: &str) {
    let (mut host, _, _, body) = StubHost::page();
    append_script(&mut host.document, body, &[], loop_body);
    let before = host.document.node_count();

    let mut rt = runtime_with(
        Limits {
            max_nodes: before + 2,
            ..Default::default()
        },
        host,
    );
    let report = rt.run_document();

    assert_eq!(report.aborted, Some(Abort::Nodes), "{report:?}");
}

#[test]
fn node_budget_limit_covers_the_textcontent_setter() {
    assert_node_budget_aborts("for (;;) { document.body.textContent = 'x'; }");
}

#[test]
fn node_budget_limit_covers_document_title() {
    assert_node_budget_aborts("for (;;) { document.title = 'x'; }");
}

#[test]
fn node_budget_limit_covers_append_with_a_string_argument() {
    assert_node_budget_aborts("for (;;) { document.body.append('x'); }");
}

#[test]
fn node_budget_limit_covers_append_with_no_arguments() {
    // `append()` with zero arguments still allocates an (empty, immediately
    // orphaned) `DocumentFragment` node every call -- this is what
    // `nodes_into_a_node`'s own `items.len() != 1` guard condition (as
    // opposed to `> 1`) exists to catch.
    assert_node_budget_aborts("for (;;) { document.body.append(); }");
}

#[test]
fn dom_content_loaded_listener_abort_is_recorded() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "document.addEventListener('DOMContentLoaded', function () { while (true) {} });",
    );

    let mut rt = runtime_with(
        Limits {
            max_loop_iterations: 10_000,
            ..Default::default()
        },
        host,
    );
    let report = rt.run_document();

    assert_eq!(report.aborted, Some(Abort::LoopIterations));
}

#[test]
fn timer_abort_during_drain_is_recorded() {
    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "setTimeout(function () { while (true) {} }, 0);",
    );

    let mut rt = runtime_with(
        Limits {
            max_loop_iterations: 10_000,
            ..Default::default()
        },
        host,
    );
    let report = rt.run_document();

    assert_eq!(report.aborted, Some(Abort::LoopIterations));
}

#[test]
fn host_failure_during_a_script_is_recorded_and_does_not_stop_the_run() {
    let (mut host, _, _, body) = StubHost::page();
    host.fail_flush = true;
    append_script(
        &mut host.document,
        body,
        &[],
        "document.body.getBoundingClientRect();",
    );

    let mut rt = DomRuntime::new(host).unwrap();
    let report = rt.run_document();

    assert_eq!(report.aborted, None, "{report:?}");
    assert_eq!(report.host_failures.len(), 1, "{report:?}");
    assert!(report.host_failures[0].contains("stub flush failure"));
    // The same failure also surfaces to script as an ordinary thrown `Error`
    // (see `RunReport::host_failures`'s own doc comment): uncaught here, it
    // is recorded in both fields at once, not just one.
    assert_eq!(report.uncaught_errors.len(), 1, "{report:?}");
    assert!(report.uncaught_errors[0].contains("stub flush failure"));
}

// ---- ScriptExecutor / RunReport plumbing -----------------------------------

#[test]
fn execute_script_is_a_no_op_for_a_non_script_or_out_of_range_node() {
    let (host, _, _, body) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let mut report = RunReport::default();
    {
        let mut executor = Executor::new(rt.context_mut(), &mut report);
        assert_eq!(
            executor.execute_script(NodeId::new(body as u64)),
            ScriptExecution::Continue
        );
        assert_eq!(
            executor.execute_script(NodeId::new(999_999)),
            ScriptExecution::Continue
        );
    }
    assert_eq!(report, RunReport::default());
}

#[test]
fn execute_script_is_a_no_op_once_the_runtime_is_aborted() {
    let (mut host, _, _, body) = StubHost::page();
    let script = append_script(&mut host.document, body, &[], "ran = true;");
    let mut rt = runtime_with(
        Limits {
            max_loop_iterations: 10_000,
            ..Default::default()
        },
        host,
    );
    let _ = rt.evaluate("while (true) {}");
    assert!(
        rt.evaluate("1").is_err(),
        "runtime should already be aborted"
    );

    let mut report = RunReport::default();
    let outcome = {
        let mut executor = Executor::new(rt.context_mut(), &mut report);
        executor.execute_script(NodeId::new(script as u64))
    };
    assert_eq!(outcome, ScriptExecution::Continue);
    assert_eq!(report, RunReport::default());
}

// ---- small helper units -----------------------------------------------------

#[test]
fn resolve_script_url_handles_absolute_relative_and_missing_base() {
    let base = "https://example.test/dir/page.html";
    assert_eq!(
        resolve_script_url(Some(base), "https://cdn.example/abs.js"),
        Some("https://cdn.example/abs.js".to_owned())
    );
    assert_eq!(
        resolve_script_url(Some(base), "ext.js"),
        Some("https://example.test/dir/ext.js".to_owned())
    );
    assert_eq!(
        resolve_script_url(Some("https://example.test"), "ext.js"),
        Some("https://example.test/ext.js".to_owned())
    );
    assert_eq!(
        resolve_script_url(Some(base), "//cdn.example/abs.js"),
        Some("https://cdn.example/abs.js".to_owned())
    );
    assert_eq!(
        resolve_script_url(Some(base), "/resources/testharness.js"),
        Some("https://example.test/resources/testharness.js".to_owned())
    );
    assert_eq!(resolve_script_url(None, "ext.js"), None);
    assert_eq!(resolve_script_url(Some("not a url"), "ext.js"), None);
}

#[test]
fn is_classic_script_type_matches_the_javascript_mime_essence_list() {
    assert!(is_classic_script_type(None));
    assert!(is_classic_script_type(Some("")));
    assert!(is_classic_script_type(Some("  ")));
    assert!(is_classic_script_type(Some("text/javascript")));
    assert!(is_classic_script_type(Some("TEXT/JAVASCRIPT")));
    assert!(!is_classic_script_type(Some("module")));
    assert!(!is_classic_script_type(Some("text/plain")));
    // The essence-match is against the fixed list of essence strings, not an
    // operation that first strips parameters from the attribute value: a
    // parameter makes the whole string not match at all, rather than being
    // stripped and ignored.
    assert!(!is_classic_script_type(Some(
        "text/javascript;charset=utf-8"
    )));
}

// ---- run_document_with_callback ----

/// A document script finds the run-scoped callback by scanning the global
/// object's symbols for its description (the delivery protocol
/// [`DomRuntime::run_document_with_callback`] exists for), claims and calls
/// it, and nothing is left behind afterwards.
#[test]
fn run_document_with_callback_installs_a_claimable_symbol_sink() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use boa_engine::JsValue;

    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "var sink = null;         var symbols = Object.getOwnPropertySymbols(globalThis);         for (var i = 0; i < symbols.length; i++) {             if (symbols[i].description === 'test sink') {                 sink = globalThis[symbols[i]];                 delete globalThis[symbols[i]];             }         }         sink('hello');",
    );
    let mut rt = DomRuntime::new(host).unwrap();
    let delivered: Rc<RefCell<Vec<String>>> = Rc::default();
    let report = rt.run_document_with_callback("test sink", {
        let delivered = Rc::clone(&delivered);
        move |_this, args, context| {
            let text = args[0].to_string(context)?.to_std_string_escaped();
            delivered.borrow_mut().push(text);
            Ok(JsValue::undefined())
        }
    });
    assert_eq!(report.aborted, None);
    assert!(report.uncaught_errors.is_empty(), "{report:?}");
    assert_eq!(report.scripts_run, 1, "{report:?}");
    assert_eq!(*delivered.borrow(), vec!["hello".to_owned()]);
    assert!(eval_bool(
        &mut rt,
        "Object.getOwnPropertySymbols(globalThis).filter(s => s.description === 'test sink')             .length === 0"
    ));
}

/// A sink no script claims is still removed after the run: nothing leaks
/// onto the global object either way.
#[test]
fn run_document_with_callback_removes_an_unclaimed_sink() {
    use boa_engine::JsValue;

    let (mut host, _, _, body) = StubHost::page();
    append_script(&mut host.document, body, &[], "var x = 1;");
    let mut rt = DomRuntime::new(host).unwrap();
    let report = rt.run_document_with_callback("test sink", |_this, _args, _context| {
        Ok(JsValue::undefined())
    });
    assert_eq!(report.scripts_run, 1, "{report:?}");
    assert!(eval_bool(
        &mut rt,
        "Object.getOwnPropertySymbols(globalThis).filter(s => s.description === 'test sink')             .length === 0"
    ));
}

/// A second call returns the cached report without running the document or
/// the callback again.
#[test]
fn run_document_with_callback_is_idempotent() {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use boa_engine::JsValue;

    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "var symbols = Object.getOwnPropertySymbols(globalThis);         for (var i = 0; i < symbols.length; i++) {             if (symbols[i].description === 'test sink') {                 globalThis[symbols[i]]();                 delete globalThis[symbols[i]];             }         }",
    );
    let mut rt = DomRuntime::new(host).unwrap();
    let runs = Rc::new(Cell::new(0u32));
    let delivered: Rc<RefCell<Vec<String>>> = Rc::default();
    let first = rt.run_document_with_callback("test sink", {
        let runs = Rc::clone(&runs);
        let delivered = Rc::clone(&delivered);
        move |_this, args, context| {
            runs.set(runs.get() + 1);
            let text = args
                .first()
                .map(|value| value.to_string(context))
                .transpose()?
                .map(|text| text.to_std_string_escaped())
                .unwrap_or_default();
            delivered.borrow_mut().push(text);
            Ok(JsValue::undefined())
        }
    });
    assert_eq!(runs.get(), 1);
    let second = rt.run_document_with_callback("test sink", |_this, _args, _context| {
        Ok(JsValue::undefined())
    });
    assert_eq!(runs.get(), 1);
    assert_eq!(first, second);
}

/// A delivery script that kept its own reference still delivers after the
/// run (the timeout-probe shape): only the symbol property is removed, the
/// callback slot stays until replaced or torn down.
#[test]
fn run_document_with_callback_keeps_a_retained_sink_callable_after_the_run() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use boa_engine::JsValue;

    let (mut host, _, _, body) = StubHost::page();
    append_script(
        &mut host.document,
        body,
        &[],
        "var stashed = null;         var symbols = Object.getOwnPropertySymbols(globalThis);         for (var i = 0; i < symbols.length; i++) {             if (symbols[i].description === 'test sink') {                 stashed = globalThis[symbols[i]];                 delete globalThis[symbols[i]];             }         }",
    );
    let mut rt = DomRuntime::new(host).unwrap();
    let delivered: Rc<RefCell<Vec<String>>> = Rc::default();
    let report = rt.run_document_with_callback("test sink", {
        let delivered = Rc::clone(&delivered);
        move |_this, args, context| {
            let text = args[0].to_string(context)?.to_std_string_escaped();
            delivered.borrow_mut().push(text);
            Ok(JsValue::undefined())
        }
    });
    assert_eq!(report.aborted, None);
    assert!(eval_bool(
        &mut rt,
        "Object.getOwnPropertySymbols(globalThis).filter(s => s.description === 'test sink')             .length === 0"
    ));
    rt.evaluate("stashed('late');").unwrap();
    assert_eq!(*delivered.borrow(), vec!["late".to_owned()]);
}

#[test]
fn body_onload_content_attribute_runs_during_run_document_like_check_layout() {
    // The check-layout smoke shape: checkLayout is invoked from
    // <body onload="...">, so the handler must be wired before the
    // document run fires its window load event.
    let (mut host, _, _, body) = StubHost::page();
    host.document
        .set_element_attribute(body, "onload", "checkLayout();")
        .unwrap();
    append_script(
        &mut host.document,
        body,
        &[],
        "var layoutCalls = 0; function checkLayout(){ layoutCalls += 1; }",
    );
    let mut rt = DomRuntime::new(host).unwrap();
    assert!(eval_bool(&mut rt, "typeof layoutCalls === 'undefined'"));
    let report = rt.run_document();
    assert_eq!(report.aborted, None, "{report:?}");
    assert!(report.uncaught_errors.is_empty(), "{report:?}");
    assert!(eval_bool(&mut rt, "layoutCalls === 1"));
    // The compiled handler is visible through both IDL getters.
    assert!(eval_bool(
        &mut rt,
        "typeof window.onload === 'function' && document.body.onload === window.onload"
    ));
}
