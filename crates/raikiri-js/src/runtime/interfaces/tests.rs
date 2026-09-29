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
         x.name === 'NotFoundError' && x.message === 'm' && x.code === 8 \
         && x instanceof Error && x instanceof DOMException \
         && x.constructor === DOMException",
    );
    // WebIDL interfaces with only optional constructor arguments have a
    // length of 0, and the prototype carries the constructor back.
    ok(
        &mut rt,
        "DOMException.length === 0 \
         && DOMException.prototype.constructor === DOMException",
    );
    ok(
        &mut rt,
        "new DOMException().name === 'Error' && new DOMException().message === '' \
         && new DOMException().code === 0",
    );
    // A single argument leaves the name at its default.
    ok(
        &mut rt,
        "new DOMException('only').message === 'only' && new DOMException('only').name === 'Error'",
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
fn dom_exception_each_legacy_name_maps_to_its_webidl_code() {
    // Every entry of the WebIDL names table (checked against the spec text
    // at https://webidl.spec.whatwg.org/#dfn-error-names-table on
    // 2026-09-29): 22 names with legacy codes, 11 new-style names with code
    // 0, and anything else (including the "Error" default) with code 0.
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "new DOMException('', 'IndexSizeError').code === 1 \
         && new DOMException('', 'HierarchyRequestError').code === 3 \
         && new DOMException('', 'WrongDocumentError').code === 4 \
         && new DOMException('', 'InvalidCharacterError').code === 5 \
         && new DOMException('', 'NoModificationAllowedError').code === 7 \
         && new DOMException('', 'NotFoundError').code === 8 \
         && new DOMException('', 'NotSupportedError').code === 9 \
         && new DOMException('', 'InUseAttributeError').code === 10 \
         && new DOMException('', 'InvalidStateError').code === 11 \
         && new DOMException('', 'SyntaxError').code === 12 \
         && new DOMException('', 'InvalidModificationError').code === 13 \
         && new DOMException('', 'NamespaceError').code === 14",
    );
    ok(
        &mut rt,
        "new DOMException('', 'InvalidAccessError').code === 15 \
         && new DOMException('', 'TypeMismatchError').code === 17 \
         && new DOMException('', 'SecurityError').code === 18 \
         && new DOMException('', 'NetworkError').code === 19 \
         && new DOMException('', 'AbortError').code === 20 \
         && new DOMException('', 'URLMismatchError').code === 21 \
         && new DOMException('', 'QuotaExceededError').code === 22 \
         && new DOMException('', 'TimeoutError').code === 23 \
         && new DOMException('', 'InvalidNodeTypeError').code === 24 \
         && new DOMException('', 'DataCloneError').code === 25",
    );
    // The new-style names from the same table carry no legacy code.
    ok(
        &mut rt,
        "new DOMException('', 'EncodingError').code === 0 \
         && new DOMException('', 'NotReadableError').code === 0 \
         && new DOMException('', 'UnknownError').code === 0 \
         && new DOMException('', 'ConstraintError').code === 0 \
         && new DOMException('', 'DataError').code === 0 \
         && new DOMException('', 'TransactionInactiveError').code === 0 \
         && new DOMException('', 'ReadOnlyError').code === 0 \
         && new DOMException('', 'VersionError').code === 0 \
         && new DOMException('', 'OperationError').code === 0 \
         && new DOMException('', 'NotAllowedError').code === 0 \
         && new DOMException('', 'OptOutError').code === 0",
    );
    // The default name and any unlisted name also read back as code 0,
    // while keeping the name itself verbatim.
    ok(
        &mut rt,
        "new DOMException('', 'Error').code === 0 \
         && new DOMException('', 'NoSuchNameError').code === 0 \
         && new DOMException('', 'NoSuchNameError').name === 'NoSuchNameError' \
         && new DOMException('m', 'NotAllowedError').name === 'NotAllowedError'",
    );
}

#[test]
fn dom_exception_all_25_idl_constants_live_on_both_objects() {
    // The IDL block in WebIDL section 4.4 declares 25 unsigned-short
    // constants; each one is defined on both the interface object and its
    // prototype.
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    ok(
        &mut rt,
        "DOMException.INDEX_SIZE_ERR === 1 && DOMException.prototype.INDEX_SIZE_ERR === 1 \
         && DOMException.DOMSTRING_SIZE_ERR === 2 && DOMException.prototype.DOMSTRING_SIZE_ERR === 2 \
         && DOMException.HIERARCHY_REQUEST_ERR === 3 && DOMException.prototype.HIERARCHY_REQUEST_ERR === 3 \
         && DOMException.WRONG_DOCUMENT_ERR === 4 && DOMException.prototype.WRONG_DOCUMENT_ERR === 4 \
         && DOMException.INVALID_CHARACTER_ERR === 5 && DOMException.prototype.INVALID_CHARACTER_ERR === 5 \
         && DOMException.NO_DATA_ALLOWED_ERR === 6 && DOMException.prototype.NO_DATA_ALLOWED_ERR === 6 \
         && DOMException.NO_MODIFICATION_ALLOWED_ERR === 7 \
         && DOMException.prototype.NO_MODIFICATION_ALLOWED_ERR === 7",
    );
    ok(
        &mut rt,
        "DOMException.NOT_FOUND_ERR === 8 && DOMException.prototype.NOT_FOUND_ERR === 8 \
         && DOMException.NOT_SUPPORTED_ERR === 9 && DOMException.prototype.NOT_SUPPORTED_ERR === 9 \
         && DOMException.INUSE_ATTRIBUTE_ERR === 10 && DOMException.prototype.INUSE_ATTRIBUTE_ERR === 10 \
         && DOMException.INVALID_STATE_ERR === 11 && DOMException.prototype.INVALID_STATE_ERR === 11 \
         && DOMException.SYNTAX_ERR === 12 && DOMException.prototype.SYNTAX_ERR === 12 \
         && DOMException.INVALID_MODIFICATION_ERR === 13 \
         && DOMException.prototype.INVALID_MODIFICATION_ERR === 13 \
         && DOMException.NAMESPACE_ERR === 14 && DOMException.prototype.NAMESPACE_ERR === 14",
    );
    ok(
        &mut rt,
        "DOMException.INVALID_ACCESS_ERR === 15 && DOMException.prototype.INVALID_ACCESS_ERR === 15 \
         && DOMException.VALIDATION_ERR === 16 && DOMException.prototype.VALIDATION_ERR === 16 \
         && DOMException.TYPE_MISMATCH_ERR === 17 && DOMException.prototype.TYPE_MISMATCH_ERR === 17 \
         && DOMException.SECURITY_ERR === 18 && DOMException.prototype.SECURITY_ERR === 18 \
         && DOMException.NETWORK_ERR === 19 && DOMException.prototype.NETWORK_ERR === 19 \
         && DOMException.ABORT_ERR === 20 && DOMException.prototype.ABORT_ERR === 20 \
         && DOMException.URL_MISMATCH_ERR === 21 && DOMException.prototype.URL_MISMATCH_ERR === 21",
    );
    ok(
        &mut rt,
        "DOMException.QUOTA_EXCEEDED_ERR === 22 && DOMException.prototype.QUOTA_EXCEEDED_ERR === 22 \
         && DOMException.TIMEOUT_ERR === 23 && DOMException.prototype.TIMEOUT_ERR === 23 \
         && DOMException.INVALID_NODE_TYPE_ERR === 24 \
         && DOMException.prototype.INVALID_NODE_TYPE_ERR === 24 \
         && DOMException.DATA_CLONE_ERR === 25 && DOMException.prototype.DATA_CLONE_ERR === 25",
    );
    // Every constant is enumerable but neither writable nor configurable,
    // on both objects.
    ok(
        &mut rt,
        "(() => { \
            const names = ['INDEX_SIZE_ERR', 'DOMSTRING_SIZE_ERR', 'HIERARCHY_REQUEST_ERR', \
                'WRONG_DOCUMENT_ERR', 'INVALID_CHARACTER_ERR', 'NO_DATA_ALLOWED_ERR', \
                'NO_MODIFICATION_ALLOWED_ERR', 'NOT_FOUND_ERR', 'NOT_SUPPORTED_ERR', \
                'INUSE_ATTRIBUTE_ERR', 'INVALID_STATE_ERR', 'SYNTAX_ERR', \
                'INVALID_MODIFICATION_ERR', 'NAMESPACE_ERR', 'INVALID_ACCESS_ERR', \
                'VALIDATION_ERR', 'TYPE_MISMATCH_ERR', 'SECURITY_ERR', 'NETWORK_ERR', \
                'ABORT_ERR', 'URL_MISMATCH_ERR', 'QUOTA_EXCEEDED_ERR', 'TIMEOUT_ERR', \
                'INVALID_NODE_TYPE_ERR', 'DATA_CLONE_ERR']; \
            return names.every((n) => [DOMException, DOMException.prototype].every((o) => { \
                const d = Object.getOwnPropertyDescriptor(o, n); \
                return d && d.enumerable && !d.writable && !d.configurable; \
            })); \
        })()",
    );
}

#[test]
fn dom_exception_thrown_values_satisfy_upstream_assert_throws_dom_checks() {
    // Upstream testharness.js assert_throws_dom_impl checks that the thrown
    // value is a non-null object whose code and name match the expected
    // entry and whose constructor is the expected global's DOMException.
    // This mirrors those checks directly so a failure here means the real
    // assert_throws_dom would also fail.
    let (host, ..) = StubHost::page();
    let mut rt = DomRuntime::new(host).unwrap();
    rt.evaluate(
        "function check_dom_throw(expectedName, expectedCode, fn) { \
            try { fn(); return false; } \
            catch (e) { \
                return typeof e === 'object' && e !== null \
                    && e instanceof DOMException \
                    && e.constructor === DOMException \
                    && e.name === expectedName \
                    && e.code === expectedCode; \
            } \
        }",
    )
    .unwrap();
    // Manually constructed values, legacy and new-style.
    ok(
        &mut rt,
        "check_dom_throw('NotFoundError', 8, function () { throw new DOMException('x', 'NotFoundError'); })",
    );
    ok(
        &mut rt,
        "check_dom_throw('NotAllowedError', 0, function () { throw new DOMException('denied', 'NotAllowedError'); })",
    );
    // Values this runtime itself throws from DOM operations.
    ok(
        &mut rt,
        "check_dom_throw('SyntaxError', 12, function () { document.querySelector('p['); })",
    );
    ok(
        &mut rt,
        "check_dom_throw('InvalidCharacterError', 5, function () { document.createElement('1bad'); })",
    );
    // The codename form upstream also accepts maps to the same entry.
    ok(
        &mut rt,
        "(() => { \
            try { document.querySelector('p['); return false; } \
            catch (e) { \
                return e.constructor === DOMException \
                    && e.code === DOMException.SYNTAX_ERR \
                    && e.code === DOMException.prototype.SYNTAX_ERR \
                    && e.name === 'SyntaxError'; \
            } \
        })()",
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
