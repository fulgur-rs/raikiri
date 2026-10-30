use super::*;
use crate::runtime::{DomRuntime, test_host::StubHost};

fn runtime() -> DomRuntime {
    DomRuntime::new(StubHost::page().0).unwrap()
}
fn ok(rt: &mut DomRuntime, script: &str) {
    assert!(rt.evaluate(script).unwrap().to_boolean());
}

#[test]
fn selective_reads_do_not_take_full_pair_snapshots() {
    let mut rt = runtime();
    ok(
        &mut rt,
        "var p=new URLSearchParams('a=one&a=two&b=three');true",
    );
    for script in [
        "p.get('a')==='one'",
        "p.has('a')",
        "p.getAll('a').join(',')==='one,two'",
    ] {
        crate::url_params_diagnostics::begin();
        ok(&mut rt, script);
        let work = crate::url_params_diagnostics::finish();
        assert_eq!(work.pairs_cloned, 0);
        assert_eq!(work.snapshots, 0);
    }
}

#[test]
fn selective_values_match_the_decoded_ordered_pair_oracle() {
    for query in [
        "",
        "?a=one&a=two&b=three",
        "space=a+b&space=a%2Bb&city=%E6%9D%B1%E4%BA%AC",
        "=empty&x=%F0%9F%8D%A3&x=%00&bad=%FF",
    ] {
        let mut rt = runtime();
        rt.evaluate(&format!("var p=new URLSearchParams({query:?});"))
            .unwrap();
        let pairs =
            url::form_urlencoded::parse(query.strip_prefix('?').unwrap_or(query).as_bytes())
                .collect::<Vec<_>>();
        for name in ["a", "b", "space", "city", "", "x", "bad", "missing"] {
            let values = pairs
                .iter()
                .filter(|(key, _)| key == name)
                .map(|(_, value)| value.to_string())
                .collect::<Vec<_>>();
            let first = values
                .first()
                .map_or("null".into(), |value| format!("{value:?}"));
            let script = format!(
                "p.get({name:?})==={first}&&p.has({name:?})==={}&&JSON.stringify(p.getAll({name:?}))===JSON.stringify({values:?})",
                !values.is_empty()
            );
            ok(&mut rt, &script);
        }
    }
}

#[test]
fn argument_conversion_precedes_invalid_receiver_and_preserves_exceptions() {
    let mut rt = runtime();
    for method in ["get", "has", "getAll"] {
        let script = format!(
            "(()=>{{let count=0;let name={{toString(){{++count;return'a';}}}};try{{URLSearchParams.prototype.{method}.call({{}},name);return false;}}catch(e){{return count===1&&e instanceof TypeError;}}}})()"
        );
        ok(&mut rt, &script);
        let script = format!(
            "(()=>{{let marker={{}};let name={{toString(){{throw marker;}}}};try{{URLSearchParams.prototype.{method}.call({{}},name);return false;}}catch(e){{return e===marker;}}}})()"
        );
        ok(&mut rt, &script);
        let script = format!(
            "(()=>{{try{{URLSearchParams.prototype.{method}.call({{}});return false;}}catch(e){{return e instanceof TypeError&&String(e).includes('name is required');}}}})()"
        );
        ok(&mut rt, &script);
    }
}

#[test]
fn name_conversion_can_reenter_the_same_params_instance() {
    let mut rt = runtime();
    for method in ["get", "has", "getAll"] {
        let script = format!(
            "(()=>{{let p=new URLSearchParams('a=one&a=two');let entered=false;let name={{toString(){{entered=p.get('a')==='one'&&p.has('a')&&p.getAll('a').length===2;return'a';}}}};let result=p.{method}(name);return entered&&({method:?}==='has'?result===true:{method:?}==='get'?result==='one':result.join(',')==='one,two');}})()"
        );
        ok(&mut rt, &script);
    }
}

#[test]
fn entries_and_symbol_iterator_keep_independent_snapshots() {
    let mut rt = runtime();
    let value = rt
        .evaluate("var p=new URLSearchParams('a=one&a=two');p")
        .unwrap();
    crate::url_params_diagnostics::begin();
    ok(
        &mut rt,
        "var iterator=p.entries();var symbolic=p[Symbol.iterator]();true",
    );
    let work = crate::url_params_diagnostics::finish();
    assert_eq!(work.snapshots, 2);
    assert_eq!(work.pairs_cloned, 4);
    {
        let object = value.as_object().unwrap();
        let mut data = object.downcast_mut::<UrlSearchParamsData>().unwrap();
        data.entries[0].1 = "changed".into();
        data.entries.push(("c".into(), "new".into()));
    }
    ok(
        &mut rt,
        "iterator.next().value[1]==='one'&&iterator.next().value[1]==='two'&&iterator.next().done&&symbolic.next().value[1]==='one'&&p.get('a')==='changed'",
    );
}
