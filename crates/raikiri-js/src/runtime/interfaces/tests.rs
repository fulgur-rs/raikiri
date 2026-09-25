use boa_engine::{Context, JsResult, JsValue};

use super::super::DomRuntime;
use super::super::test_host::StubHost;
use super::{Members, interface};

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
    interface(rt.context_mut(), "Probe", None, None, &PROBE).unwrap();
    let check = "(() => { \
        const d = Object.getOwnPropertyDescriptor(Probe.prototype, 'value'); \
        const m = Probe.prototype.count; \
        return d.enumerable && d.configurable \
            && d.get.name === 'get value' && d.set.name === 'set value' \
            && d.get.call() === 1 && d.set.call(null, 5) === 5 \
            && m.length === 2 && m(1, 2, 3) === 3; \
    })()";
    assert!(rt.evaluate(check).unwrap().to_boolean());
}
