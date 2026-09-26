use super::test_host::StubHost;
use super::{DomRuntime, RuntimeError};

fn eval_bool(runtime: &mut DomRuntime, source: &str) -> bool {
    runtime.evaluate(source).unwrap().to_boolean()
}

#[test]
fn interface_objects_form_the_dom_prototype_chain() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    assert!(eval_bool(
        &mut rt,
        "Object.getPrototypeOf(HTMLElement.prototype) === Element.prototype"
    ));
    assert!(eval_bool(
        &mut rt,
        "Object.getPrototypeOf(Element.prototype) === Node.prototype"
    ));
    assert!(eval_bool(
        &mut rt,
        "Object.getPrototypeOf(Node.prototype) === EventTarget.prototype"
    ));
    assert!(eval_bool(
        &mut rt,
        "Object.getPrototypeOf(Document.prototype) === Node.prototype"
    ));
    assert!(eval_bool(
        &mut rt,
        "Object.getPrototypeOf(Text.prototype) === CharacterData.prototype"
    ));
    assert!(eval_bool(
        &mut rt,
        "Object.getPrototypeOf(HTMLElement) === Element"
    ));
    assert!(eval_bool(
        &mut rt,
        "document instanceof Document && document instanceof Node && document instanceof EventTarget"
    ));
    assert!(eval_bool(
        &mut rt,
        "window === globalThis && self === globalThis"
    ));
}

#[test]
fn interface_constructors_are_illegal() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    assert!(eval_bool(
        &mut rt,
        "try { new Node(); false } catch (e) { e instanceof TypeError }"
    ));
    assert!(eval_bool(
        &mut rt,
        "try { Element(); false } catch (e) { e instanceof TypeError }"
    ));
}

#[test]
fn global_object_exposes_no_internal_names() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    assert!(eval_bool(
        &mut rt,
        "!Object.getOwnPropertyNames(globalThis).some(n => n.startsWith('__raikiri'))"
    ));
}

#[test]
fn into_host_returns_the_same_document() {
    let (host, _, _, body) = StubHost::page();
    let rt = DomRuntime::new(host).unwrap();
    let host = rt.into_host();
    assert_eq!(
        host.document().get_node(body).and_then(|n| n.tag_name()),
        Some("body")
    );
}

#[test]
fn document_wrapper_is_stable_and_keeps_expandos() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate("document.foo = 42;").unwrap();
    assert!(eval_bool(
        &mut rt,
        "document.foo === 42 && window.document === document"
    ));
}

#[test]
fn brand_mismatch_throws_type_error_and_runtime_keeps_working() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let err =
        rt.evaluate("Object.getOwnPropertyDescriptor(Node.prototype, 'nodeType').get.call({})");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "{err:?}"
    );
    assert!(eval_bool(&mut rt, "document.nodeType === 9"));
}

use boa_engine::property::Attribute;
use boa_engine::{JsObject, JsValue, js_string};

use super::host::{DocumentHost, HostError};
use super::interfaces::{NodeHandle, protos, wrap, wrap_optional};
use super::webidl::{
    arg_node, dom_string, host_failure, this_document, this_element, this_node,
    throw_dom_exception, with_state,
};
use super::{BoxGeometry, shared};

/// Arena indices of one node of every kind, beyond the page skeleton.
struct Kinds {
    body: usize,
    text: usize,
    comment: usize,
    pi: usize,
    fragment: usize,
    svg: usize,
}

fn runtime_with_every_node_kind() -> (DomRuntime, Kinds) {
    let (mut host, _, _, body) = StubHost::page();
    let doc = &mut host.document;
    let text = doc.append_text(body, "hi");
    let comment = doc.append_comment(Some(body), "note");
    let pi = doc.append_processing_instruction(Some(body), "target", "data");
    let template = doc.create_detached_element("template").unwrap();
    doc.append_child(body, template).unwrap();
    let fragment = doc.allocate_template_fragment_root(template);
    let svg = doc.create_detached_element("svg").unwrap();
    doc.set_element_namespace(svg, Some("http://www.w3.org/2000/svg".into()));
    doc.append_child(body, svg).unwrap();
    let rt = DomRuntime::new(host).unwrap();
    (
        rt,
        Kinds {
            body,
            text,
            comment,
            pi,
            fragment,
            svg,
        },
    )
}

/// Expose `value` to scripts as global `name`.
fn expose(rt: &mut DomRuntime, name: &str, value: impl Into<JsValue>) {
    rt.context_mut()
        .register_global_property(boa_engine::JsString::from(name), value, Attribute::all())
        .unwrap();
}

fn expose_node(rt: &mut DomRuntime, name: &str, index: usize) {
    let object = wrap(rt.context_mut(), index).unwrap();
    expose(rt, name, object);
}

/// A wrapper-shaped object whose index is past the end of the arena.
fn dangling_handle(rt: &mut DomRuntime) -> JsObject {
    let proto = protos(rt.context_mut()).node.clone();
    JsObject::from_proto_and_data(Some(proto), NodeHandle { index: 9999 })
}

#[test]
fn wrappers_pick_the_interface_of_their_node_kind() {
    let (mut rt, k) = runtime_with_every_node_kind();
    for (name, index) in [
        ("body", k.body),
        ("text", k.text),
        ("comment", k.comment),
        ("pi", k.pi),
        ("fragment", k.fragment),
        ("svg", k.svg),
    ] {
        expose_node(&mut rt, name, index);
    }
    assert!(eval_bool(
        &mut rt,
        "body instanceof HTMLElement && body.nodeType === 1"
    ));
    assert!(eval_bool(
        &mut rt,
        "svg instanceof Element && !(svg instanceof HTMLElement) && svg.nodeType === 1"
    ));
    assert!(eval_bool(
        &mut rt,
        "text instanceof Text && text instanceof CharacterData && text.nodeType === 3"
    ));
    assert!(eval_bool(
        &mut rt,
        "comment instanceof Comment && comment.nodeType === 8"
    ));
    assert!(eval_bool(
        &mut rt,
        "pi instanceof ProcessingInstruction && pi instanceof CharacterData && pi.nodeType === 7"
    ));
    assert!(eval_bool(
        &mut rt,
        "fragment instanceof DocumentFragment && fragment.nodeType === 11"
    ));
}

#[test]
fn wrap_is_identity_preserving_and_grows_with_the_arena() {
    let (mut rt, k) = runtime_with_every_node_kind();
    let ctx = rt.context_mut();
    let first = wrap(ctx, k.body).unwrap();
    assert!(JsObject::equals(&first, &wrap(ctx, k.body).unwrap()));
    // A node created after the runtime lies past the initial wrapper table.
    let late = with_state(ctx, |s| {
        s.host
            .document_mut()
            .create_detached_element("div")
            .unwrap()
    })
    .unwrap();
    let late_wrapper = wrap(ctx, late).unwrap();
    assert!(JsObject::equals(&late_wrapper, &wrap(ctx, late).unwrap()));
    assert!(wrap(ctx, 9999).is_err());
    assert!(wrap_optional(ctx, None).unwrap().is_null());
    let some = wrap_optional(ctx, Some(k.body)).unwrap();
    assert!(JsObject::equals(&some.as_object().unwrap(), &first));
}

#[test]
fn brand_checks_distinguish_interfaces() {
    let (mut rt, k) = runtime_with_every_node_kind();
    let dangling: JsValue = dangling_handle(&mut rt).into();
    let ctx = rt.context_mut();
    let root = with_state(ctx, |s| s.host.document().root_index()).unwrap();
    let document: JsValue = wrap(ctx, root).unwrap().into();
    let body: JsValue = wrap(ctx, k.body).unwrap().into();
    let plain: JsValue = JsObject::with_null_proto().into();

    assert_eq!(this_node(&body, ctx).unwrap(), k.body);
    assert!(this_node(&plain, ctx).is_err());
    assert!(this_node(&JsValue::from(1), ctx).is_err());
    assert_eq!(this_element(&body, ctx).unwrap(), k.body);
    assert!(this_element(&document, ctx).is_err());
    assert!(this_element(&dangling, ctx).is_err());
    assert_eq!(this_document(&document, ctx).unwrap(), root);
    assert!(this_document(&body, ctx).is_err());

    let args = [body.clone(), plain];
    assert_eq!(arg_node(&args, 0, ctx).unwrap(), k.body);
    assert!(arg_node(&args, 1, ctx).is_err());
    assert!(arg_node(&args, 2, ctx).is_err());
}

#[test]
fn node_type_of_a_dangling_handle_is_zero() {
    let (mut rt, _) = runtime_with_every_node_kind();
    let dangling = dangling_handle(&mut rt);
    expose(&mut rt, "dangling", dangling);
    assert!(eval_bool(&mut rt, "dangling.nodeType === 0"));
}

#[test]
fn dom_string_applies_to_string() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let ctx = rt.context_mut();
    let args = [JsValue::from(12), JsValue::from(js_string!("x"))];
    assert_eq!(dom_string(&args, 0, ctx).unwrap(), "12");
    assert_eq!(dom_string(&args, 1, ctx).unwrap(), "x");
    assert_eq!(dom_string(&args, 2, ctx).unwrap(), "undefined");
    let symbol = [JsValue::from(boa_engine::JsSymbol::new(None).unwrap())];
    assert!(dom_string(&symbol, 0, ctx).is_err());
}

#[test]
fn dom_exceptions_are_errors_with_name_message_and_code() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    for (i, name) in [
        "HierarchyRequestError",
        "InvalidCharacterError",
        "NotFoundError",
        "SyntaxError",
        "OtherError",
    ]
    .into_iter()
    .enumerate()
    {
        let error = throw_dom_exception(rt.context_mut(), name, "details");
        let value = error.into_opaque(rt.context_mut()).unwrap();
        expose(&mut rt, &format!("ex{i}"), value);
    }
    assert!(eval_bool(
        &mut rt,
        "ex0 instanceof DOMException && ex0 instanceof Error \
         && ex0.name === 'HierarchyRequestError' && ex0.message === 'details'"
    ));
    assert!(eval_bool(
        &mut rt,
        "[ex0, ex1, ex2, ex3, ex4].map(e => e.code).join() === '3,5,8,12,0'"
    ));
    assert!(eval_bool(
        &mut rt,
        "Object.getPrototypeOf(DOMException) === Function.prototype"
    ));
    assert!(eval_bool(
        &mut rt,
        "try { Object.getOwnPropertyDescriptor(DOMException.prototype, 'name').get.call({}); false } \
         catch (e) { e instanceof TypeError }"
    ));
    // An uncaught non-native exception is reported through its string form.
    let err = rt.evaluate("throw ex2");
    assert!(matches!(err, Err(RuntimeError::JavaScript(_))), "{err:?}");
    let err = rt.evaluate("throw 'plain'");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.starts_with("\"plain\"")),
        "{err:?}"
    );
}

/// An uncaught `DOMException` is an opaque object (its data lives in Rust
/// state, not in Boa's own error representation), so `try_native` fails for
/// it; the reported message must still run `Error.prototype.toString`
/// (`Name: message`) rather than falling back to an opaque object dump.
#[test]
fn uncaught_dom_exception_message_reports_name_and_message() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let err = rt.evaluate("document.body.appendChild(document);");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("HierarchyRequestError")),
        "{err:?}"
    );
}

#[test]
fn host_failures_take_precedence_over_script_results() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let error = host_failure(rt.context_mut(), HostError("layout exploded".into()));
    assert!(error.to_string().contains("layout exploded"));
    // Only the first failure of an evaluation is kept.
    let _ = host_failure(rt.context_mut(), HostError("second".into()));
    assert_eq!(
        rt.evaluate("1"),
        Err(RuntimeError::Host("layout exploded".into()))
    );
    // The failure is consumed by the evaluation that reported it.
    assert_eq!(rt.evaluate("1 + 1").unwrap(), JsValue::from(2));
}

#[test]
fn reentrant_state_access_is_an_exception_and_a_host_failure() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    let state = shared(rt.context_mut());
    {
        let _held = state.0.borrow_mut();
        let ctx = rt.context_mut();
        let err = with_state(ctx, |_| ()).unwrap_err();
        assert!(err.to_string().contains("re-entrant DOM runtime access"));
        // Later failures while the first is parked are not recorded.
        let _ = host_failure(ctx, HostError("ignored".into()));
    }
    // A failure recorded after the borrow ends does not displace the parked one.
    let _ = host_failure(rt.context_mut(), HostError("later".into()));
    assert_eq!(
        rt.evaluate("1"),
        Err(RuntimeError::Host("re-entrant DOM runtime access".into()))
    );
    assert!(rt.evaluate("1").is_ok());
}

#[test]
fn errors_display_their_messages() {
    assert_eq!(HostError("boom".into()).to_string(), "boom");
    assert_eq!(
        RuntimeError::JavaScript("TypeError: x".into()).to_string(),
        "JavaScript error: TypeError: x"
    );
    assert_eq!(
        RuntimeError::Host("boom".into()).to_string(),
        "host error: boom"
    );
}

#[test]
fn stub_host_reports_configured_values() {
    let (mut host, _, _, body) = StubHost::page();
    host.geometry.insert(body, StubHost::rect(5.0));
    host.computed
        .insert((body, "display".into()), "block".into());
    assert_eq!(
        host.box_geometry(body),
        Ok(Some(BoxGeometry {
            border_box: super::DomRect {
                left: 1.0,
                top: 2.0,
                right: 11.0,
                bottom: 7.0,
                width: 10.0,
                height: 5.0,
            },
        }))
    );
    assert_eq!(host.box_geometry(0), Ok(None));
    assert_eq!(
        host.computed_value(body, "display"),
        Ok(Some("block".into()))
    );
    assert_eq!(host.computed_value(body, "color"), Ok(None));
    assert_eq!(host.flush(), Ok(()));
    host.fail_flush = true;
    assert_eq!(host.flush(), Err(HostError("stub flush failure".into())));
    assert_eq!(host.flushes.get(), 2);
    let fragment = host.parse_fragment("div", "", "<b>x</b>").unwrap();
    let root = fragment.root_index();
    let child = fragment.get_node(root).unwrap().children[0];
    assert_eq!(
        fragment.get_node(child).and_then(|n| n.text_content()),
        Some("<b>x</b>")
    );
    let _ = host.document_mut();
}
