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
        "s1:true:loading|s2:true:loading|s3:true:loading|dcl:interactive|load:complete"
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

    assert_eq!(report.scripts_run, 2, "{report:?}");
    assert_eq!(
        eval_string(&mut rt, "ran.join(',')"),
        "no-type,classic-with-params"
    );
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
             events.push('missing-error:' + e.bubbles + ':' + e.cancelable + ':' + e.isTrusted); \
         }); \
         empty_src.addEventListener('error', function (e) { events.push('empty-error'); });",
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
            ": the src attribute is empty".to_owned(),
        ],
        "{report:?}"
    );
    assert_eq!(
        eval_string(&mut rt, "events.join('|')"),
        "missing-error:false:false:true|empty-error|after"
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
    assert_eq!(resolve_script_url(None, "ext.js"), None);
    assert_eq!(resolve_script_url(Some("not a url"), "ext.js"), None);
}

#[test]
fn is_classic_script_type_matches_the_javascript_mime_essence_list() {
    assert!(is_classic_script_type(None));
    assert!(is_classic_script_type(Some("")));
    assert!(is_classic_script_type(Some("  ")));
    assert!(is_classic_script_type(Some("text/javascript")));
    assert!(is_classic_script_type(Some(
        "TEXT/JAVASCRIPT;charset=utf-8"
    )));
    assert!(!is_classic_script_type(Some("module")));
    assert!(!is_classic_script_type(Some("text/plain")));
}
