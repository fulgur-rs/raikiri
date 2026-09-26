use boa_engine::{Context, JsResult, JsValue};

use super::super::DomRuntime;
use super::super::test_host::StubHost;
use super::{Members, illegal_constructor, interface};

fn probe_get(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(1))
}

fn probe_set(_: &JsValue, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(args.first().cloned().unwrap_or_default())
}

fn probe_method(_: &JsValue, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(args.len() as i32))
}

const PROBE: Members = Members {
    getters: &[],
    accessors: &[("value", probe_get, probe_set)],
    methods: &[("count", 2, probe_method)],
};

#[test]
fn interface_members_become_prototype_accessors_and_operations() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    interface(
        rt.context_mut(),
        "Probe",
        None,
        None,
        &[&PROBE],
        illegal_constructor,
        0,
    )
    .unwrap();
    let check = "(() => { \
        const d = Object.getOwnPropertyDescriptor(Probe.prototype, 'value'); \
        const m = Probe.prototype.count; \
        return d.enumerable && d.configurable \
            && d.get.name === 'get value' && d.set.name === 'set value' \
            && d.get.call() === 1 && d.set.call(null, 5) === 5 \
            && m.length === 2 && m(1, 2, 3) === 3 \
            && d.get.length === 0 && d.set.length === 1; \
    })()";
    assert!(rt.evaluate(check).unwrap().to_boolean());
    let operation = "(() => { \
        const d = Object.getOwnPropertyDescriptor(Probe.prototype, 'count'); \
        const l = Object.getOwnPropertyDescriptor(d.value, 'length'); \
        return d.enumerable && d.writable && d.configurable && d.value.name === 'count' \
            && l.value === 2 && !l.writable && !l.enumerable && l.configurable; \
    })()";
    assert!(rt.evaluate(operation).unwrap().to_boolean());
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

#[test]
fn dom_exception_constructor_takes_message_and_name_with_defaults() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "var x = new DOMException('m', 'NotFoundError'); \
         x.name === 'NotFoundError' && x.message === 'm' && x.code === 8 && x instanceof Error",
    );
    ok(
        &mut rt,
        "new DOMException().name === 'Error' && new DOMException().message === '' \
         && new DOMException().code === 0",
    );
    // Explicit `undefined` for either optional argument takes the same
    // default as a genuinely missing one, per WebIDL optional-with-default
    // conversion (not a plain `ToString(undefined)` = `"undefined"`).
    ok(
        &mut rt,
        "var y = new DOMException(undefined, undefined); y.message === '' && y.name === 'Error'",
    );
}

#[test]
fn dom_exception_full_legacy_code_table_and_constants() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "new DOMException('', 'IndexSizeError').code === 1 \
         && new DOMException('', 'InvalidStateError').code === 11 \
         && new DOMException('', 'DataCloneError').code === 25 \
         && new DOMException('', 'NoSuchNameError').code === 0",
    );
    ok(
        &mut rt,
        "DOMException.NOT_FOUND_ERR === 8 && DOMException.prototype.SYNTAX_ERR === 12 \
         && DOMException.INUSE_ATTRIBUTE_ERR === 10 && DOMException.prototype.DATA_CLONE_ERR === 25",
    );
    ok(
        &mut rt,
        "(() => { \
            const d = Object.getOwnPropertyDescriptor(DOMException, 'NOT_FOUND_ERR'); \
            return d.enumerable && !d.writable && !d.configurable; \
        })()",
    );
    // The three legacy constants with no corresponding error name (WebIDL
    // §2.8.1's `DOMException` IDL block declares 25 constants total, not
    // just the ones this runtime's own thrown names use) are still defined
    // on both the interface object and its prototype.
    ok(
        &mut rt,
        "DOMException.DOMSTRING_SIZE_ERR === 2 && DOMException.prototype.DOMSTRING_SIZE_ERR === 2 \
         && DOMException.NO_DATA_ALLOWED_ERR === 6 && DOMException.prototype.NO_DATA_ALLOWED_ERR === 6 \
         && DOMException.VALIDATION_ERR === 16 && DOMException.prototype.VALIDATION_ERR === 16",
    );
}

#[test]
fn dom_exception_requires_new() {
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "try { DOMException('m', 'Error'); false } catch (e) { e instanceof TypeError }",
    );
}
