use crate::runtime::DomRuntime;
use crate::runtime::test_host::StubHost;

thread_local! {
    static WORK: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

pub(super) fn record_visit(stack: usize) {
    WORK.with(|work| {
        let (visits, peak) = work.get();
        work.set((visits + 1, peak.max(stack)));
    });
}

fn reset_work() {
    WORK.with(|work| work.set((0, 0)));
}

fn work() -> (usize, usize) {
    WORK.with(std::cell::Cell::get)
}

fn yes(runtime: &mut DomRuntime, script: &str) {
    let result = runtime
        .evaluate(script)
        .unwrap_or_else(|error| panic!("{script}: {error:?}"));
    assert!(result.to_boolean(), "{script}");
}

#[test]
fn repeated_named_miss_walks_once_with_depth_bounded_storage() {
    let (mut host, _, _, body) = StubHost::page();
    for _ in 0..256 {
        host.document
            .append_element(Some(body), "div", None::<&str>);
    }
    let count = host.document.node_count();
    let mut runtime = DomRuntime::new(host).unwrap();
    reset_work();
    yes(
        &mut runtime,
        "(() => { for (let i=0; i<100; ++i) { if ('__missing996__' in window || window.__missing996__ !== undefined) return false; } return true; })()",
    );
    assert_eq!(work().0, count);
    assert!(work().1 <= 4);
}

#[test]
fn has_stops_at_first_match_and_get_classifies_at_second() {
    let (mut host, _, _, body) = StubHost::page();
    for _ in 0..2 {
        let node = host
            .document
            .append_element(Some(body), "div", None::<&str>);
        host.document
            .set_element_attribute(node, "id", "__group996__")
            .unwrap();
    }
    for _ in 0..256 {
        host.document
            .append_element(Some(body), "div", None::<&str>);
    }
    let count = host.document.node_count();
    let mut runtime = DomRuntime::new(host).unwrap();
    reset_work();
    yes(&mut runtime, "'__group996__' in window");
    assert_eq!(work().0, 5);
    reset_work();
    yes(
        &mut runtime,
        "window.__group996__ instanceof HTMLCollection",
    );
    assert_eq!(work().0, 6);
    reset_work();
    yes(
        &mut runtime,
        "var named996 = window.__group996__; named996.length === 2",
    );
    assert_eq!(work().0, count);
    reset_work();
    yes(
        &mut runtime,
        "named996.length === 2 && '__group996__' in window && window.__group996__ instanceof HTMLCollection",
    );
    assert_eq!(work().0, 0);
}

#[test]
#[ignore = "manual release-mode Window named lookup measurement"]
fn window_named_miss_scaling_probe() {
    use std::time::Instant;
    let cases = [
        (
            "miss",
            "(() => { let found = false; for (let i = 0; i < 1000; ++i) { found ||= ('__perf996_missing__' in window); } return found; })()",
            false,
        ),
        (
            "has_first",
            "(() => { let found = true; for (let i = 0; i < 1000; ++i) { found &&= ('__perf996_hit__' in window); } return found; })()",
            true,
        ),
        (
            "get_two",
            "(() => { let found = true; for (let i = 0; i < 1000; ++i) { found &&= (window.__perf996_hit__ instanceof HTMLCollection); } return found; })()",
            true,
        ),
    ];
    for (mode, source, expected) in cases {
        for n in [1_000, 2_000, 4_000, 8_000] {
            let mut samples = Vec::new();
            for _ in 0..5 {
                let (mut host, _, _, body) = StubHost::page();
                for i in 0..n {
                    let node = host
                        .document
                        .append_element(Some(body), "div", None::<&str>);
                    if (mode == "has_first" && i == 0) || (mode == "get_two" && i < 2) {
                        host.document
                            .set_element_attribute(node, "id", "__perf996_hit__")
                            .unwrap();
                    }
                }
                let mut runtime = DomRuntime::new(host).unwrap();
                let start = Instant::now();
                let result = runtime.evaluate(source).unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1_000.0);
                assert_eq!(result.to_boolean(), expected);
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "window named mode={mode} n={n} queries=1000 median_ms={:.6} samples={samples:?}",
                samples[2]
            );
        }
    }
}

#[test]
fn cache_bounds_key_memory_and_eviction_preserves_results() {
    use super::*;
    let mut document = Document::new();
    let node = document.append_element(Some(0), "div", None::<&str>);
    document
        .set_element_attribute(node, "id", "present")
        .unwrap();
    let mut cache = NamedAccessCache::default();
    assert!(cache.has(&document, "present"));
    for i in 1..MAX_CACHE_ENTRIES {
        assert!(!cache.has(&document, &format!("missing-{i}")));
    }
    assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
    let bytes = cache.key_bytes;
    assert_eq!(cache.get(&document, "present"), Matches::Single(node));
    assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
    assert_eq!(cache.key_bytes, bytes);
    assert!(!cache.has(&document, "eviction"));
    assert!(cache.entries.len() < MAX_CACHE_ENTRIES);
    reset_work();
    assert!(cache.has(&document, "present"));
    assert_eq!(work().0, document.node_count());
    let long = "é".repeat(MAX_SINGLE_KEY_BYTES / 2);
    for i in 0..(MAX_CACHE_KEY_BYTES / MAX_SINGLE_KEY_BYTES + 2) {
        let name = format!("{i:04}{}", &long[..long.len() - 4]);
        assert!(!cache.has(&document, &name));
        assert!(cache.key_bytes <= MAX_CACHE_KEY_BYTES);
        assert!(cache.entries.len() <= MAX_CACHE_ENTRIES);
        assert_eq!(
            cache.key_bytes,
            cache.entries.keys().map(String::len).sum::<usize>()
        );
    }
    let oversized = "x".repeat(MAX_SINGLE_KEY_BYTES + 1);
    let bytes = cache.key_bytes;
    reset_work();
    assert!(!cache.has(&document, &oversized));
    assert_eq!(cache.get(&document, &oversized), Matches::Missing);
    assert_eq!(work().0, 2 * document.node_count());
    assert!(!cache.entries.contains_key(&oversized));
    assert_eq!(cache.key_bytes, bytes);
    reset_work();
    assert!(!cache.has(&document, ""));
    assert_eq!(cache.get(&document, ""), Matches::Missing);
    assert_eq!(work().0, 0);
}

#[test]
fn cached_single_and_multiple_results_follow_js_mutations_and_tree_order() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    yes(&mut runtime, "typeof window.__live996__ === 'undefined'");
    yes(
        &mut runtime,
        "var first996 = document.createElement('form'); first996.id = '__live996__'; first996.setAttribute('name', '__live996__'); document.body.append(first996); window.__live996__ === first996 && ('__live996__' in window)",
    );
    reset_work();
    yes(
        &mut runtime,
        "window.__live996__ === first996 && ('__live996__' in window)",
    );
    assert_eq!(work().0, 0);
    yes(
        &mut runtime,
        "var second996 = document.createElement('form'); second996.setAttribute('name', '__live996__'); document.body.append(second996); var group996 = window.__live996__; group996.length === 2 && group996[0] === first996 && group996[1] === second996",
    );
    yes(
        &mut runtime,
        "document.body.insertBefore(second996, first996); group996[0] === second996 && group996[1] === first996",
    );
    yes(
        &mut runtime,
        "second996.remove(); group996.length === 1 && window.__live996__ === first996",
    );
    yes(
        &mut runtime,
        "first996.id = 'renamed996'; first996.removeAttribute('name'); group996.length === 0 && !('__live996__' in window)",
    );
    yes(
        &mut runtime,
        "second996.removeAttribute('name'); second996.id = '__live996__'; document.body.append(second996); window.__live996__ === second996 && group996.length === 1",
    );
}

#[test]
fn cache_keeps_prototype_fallback_live_and_never_shadows_named_nodes() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    yes(
        &mut runtime,
        "window.__prototype996__ === undefined && !('__prototype996__' in window)",
    );
    yes(
        &mut runtime,
        "Object.prototype.__prototype996__ = 23; window.__prototype996__ === 23 && ('__prototype996__' in window)",
    );
    yes(
        &mut runtime,
        "delete Object.prototype.__prototype996__; window.__prototype996__ === undefined && !('__prototype996__' in window)",
    );
    yes(
        &mut runtime,
        "var node996 = document.createElement('div'); node996.id = '__prototype996__'; document.body.append(node996); Object.prototype.__prototype996__ = 23; window.__prototype996__ === node996",
    );
    yes(
        &mut runtime,
        "delete Object.prototype.__prototype996__; window.__prototype996__ === node996",
    );
}

#[test]
fn native_changes_refresh_retained_named_collections_without_js_generation() {
    use crate::runtime::webidl::with_state;
    let (mut host, _, _, body) = StubHost::page();
    let first = host
        .document
        .append_element(Some(body), "form", None::<&str>);
    let second = host
        .document
        .append_element(Some(body), "form", None::<&str>);
    for node in [first, second] {
        host.document
            .set_element_attribute(node, "name", "__native996__")
            .unwrap();
    }
    let mut runtime = DomRuntime::new(host).unwrap();
    yes(
        &mut runtime,
        "var retained996 = window.__native996__; retained996.length === 2",
    );
    let generation = with_state(runtime.context_mut(), |s| s.generation).unwrap();
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .pre_insert(body, second, Some(first))
            .unwrap();
    })
    .unwrap();
    reset_work();
    yes(
        &mut runtime,
        "retained996.length === 2 && retained996[0] === document.body.firstElementChild",
    );
    assert!(work().0 > 0);
    reset_work();
    yes(&mut runtime, "retained996.length === 2");
    assert_eq!(work().0, 0);
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .remove_element_attribute_ns(first, None, "name");
    })
    .unwrap();
    yes(
        &mut runtime,
        "retained996.length === 1 && window.__native996__ === retained996[0]",
    );
    with_state(runtime.context_mut(), |s| {
        s.host
            .document_mut()
            .set_element_namespace(second, Some("http://www.w3.org/2000/svg".into()));
    })
    .unwrap();
    yes(
        &mut runtime,
        "retained996.length === 0 && !('__native996__' in window)",
    );
    with_state(runtime.context_mut(), |s| {
        s.host.document_mut().set_element_namespace(second, None);
    })
    .unwrap();
    yes(
        &mut runtime,
        "retained996.length === 1 && ('__native996__' in window)",
    );
    with_state(runtime.context_mut(), |s| {
        s.host.document_mut().detach_from_parent(second);
    })
    .unwrap();
    yes(
        &mut runtime,
        "retained996.length === 0 && window.__native996__ === undefined",
    );
    with_state(runtime.context_mut(), |s| {
        s.host.document_mut().attach_child(body, second);
        s.host
            .document_mut()
            .set_element_attributes(second, vec![("id".into(), "__native996__".into())]);
    })
    .unwrap();
    yes(
        &mut runtime,
        "retained996.length === 1 && ('__native996__' in window)",
    );
    assert_eq!(
        with_state(runtime.context_mut(), |s| s.generation).unwrap(),
        generation
    );
}

#[test]
fn cached_node_id_does_not_keep_detached_wrapper_or_payload_alive() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    runtime.evaluate("var weakNamed996; var weakPayload996; (() => { let node = document.createElement('div'); node.id = '__gc996__'; node.payload = new Uint8Array(1048576); document.body.append(node); if (window.__gc996__ !== node) throw new Error('wrong named node'); weakNamed996 = new WeakRef(node); weakPayload996 = new WeakRef(node.payload); node.remove(); })()").unwrap();
    runtime.context_mut().clear_kept_objects();
    boa_gc::force_collect();
    yes(
        &mut runtime,
        "weakNamed996.deref() === undefined && weakPayload996.deref() === undefined && !('__gc996__' in window)",
    );
}

struct RenamingFlushHost {
    inner: StubHost,
    node: usize,
    rename: Option<&'static str>,
}

impl crate::runtime::host::DocumentHost for RenamingFlushHost {
    fn document(&self) -> &raikiri_dom::Document {
        &self.inner.document
    }
    fn document_mut(&mut self) -> &mut raikiri_dom::Document {
        &mut self.inner.document
    }
    fn flush(&mut self) -> Result<(), crate::runtime::host::HostError> {
        if let Some(name) = self.rename.take() {
            self.inner
                .document
                .set_element_attribute(self.node, "id", name)
                .unwrap();
        }
        self.inner.flush()
    }
    fn box_geometry(
        &mut self,
        node: usize,
    ) -> Result<Option<crate::runtime::host::BoxGeometry>, crate::runtime::host::HostError> {
        self.inner.box_geometry(node)
    }
    fn computed_value(
        &mut self,
        node: usize,
        property: &str,
    ) -> Result<Option<String>, crate::runtime::host::HostError> {
        self.inner.computed_value(node, property)
    }
    fn parse_fragment(
        &mut self,
        tag: &str,
        namespace: &str,
        markup: &str,
    ) -> Result<raikiri_dom::Document, crate::runtime::host::HostError> {
        self.inner.parse_fragment(tag, namespace, markup)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

#[test]
fn native_flush_renaming_invalidates_both_cached_miss_and_hit() {
    use crate::runtime::webidl::with_state;
    let (mut host, _, _, body) = StubHost::page();
    let node = host
        .document
        .append_element(Some(body), "div", None::<&str>);
    host.document
        .set_element_attribute(node, "id", "__beforeFlush996__")
        .unwrap();
    let mut runtime = DomRuntime::new(RenamingFlushHost {
        inner: host,
        node,
        rename: Some("__afterFlush996__"),
    })
    .unwrap();
    let generation = with_state(runtime.context_mut(), |s| s.generation).unwrap();
    yes(
        &mut runtime,
        "!('__afterFlush996__' in window) && ('__beforeFlush996__' in window)",
    );
    yes(
        &mut runtime,
        "document.getElementById('__beforeFlush996__').getBoundingClientRect(); ('__afterFlush996__' in window) && !('__beforeFlush996__' in window)",
    );
    assert_eq!(
        with_state(runtime.context_mut(), |s| s.generation).unwrap(),
        generation
    );
}

#[test]
fn layout_flush_without_source_changes_reuses_named_miss() {
    let (host, ..) = StubHost::page();
    let flushes = host.flushes.clone();
    let mut runtime = DomRuntime::new(host).unwrap();
    yes(&mut runtime, "!('__flushMiss996__' in window)");
    reset_work();
    yes(
        &mut runtime,
        "document.body.getBoundingClientRect(); !('__flushMiss996__' in window)",
    );
    assert_eq!(flushes.get(), 1);
    assert_eq!(work().0, 0);
}

#[test]
fn cache_respects_name_tag_namespace_and_template_scope() {
    let (host, ..) = StubHost::page();
    let mut runtime = DomRuntime::new(host).unwrap();
    yes(
        &mut runtime,
        "var namedForm996 = document.createElement('form'); namedForm996.setAttribute('name', '__scope996__'); document.body.append(namedForm996); window.__scope996__ === namedForm996",
    );
    yes(
        &mut runtime,
        "var namedDiv996 = document.createElement('div'); namedDiv996.setAttribute('name', '__scope996__'); document.body.append(namedDiv996); window.__scope996__ === namedForm996",
    );
    yes(
        &mut runtime,
        "namedForm996.remove(); window.__scope996__ === undefined && !('__scope996__' in window)",
    );
    yes(
        &mut runtime,
        "namedDiv996.id = '__scope996__'; window.__scope996__ === namedDiv996",
    );
    yes(
        &mut runtime,
        "var foreign996 = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); foreign996.id = '__scope996__'; document.body.append(foreign996); window.__scope996__ === namedDiv996",
    );
    yes(
        &mut runtime,
        "var template996 = document.createElement('template'); document.body.append(template996); window.__scope996__ === namedDiv996",
    );
    let template =
        crate::runtime::webidl::node_index(&runtime.evaluate("template996").unwrap()).unwrap();
    let moving =
        crate::runtime::webidl::node_index(&runtime.evaluate("namedDiv996").unwrap()).unwrap();
    crate::runtime::webidl::with_state(runtime.context_mut(), |s| {
        let doc = s.host.document_mut();
        let contents = doc.get_node(template).unwrap().template_contents().unwrap();
        doc.pre_insert(contents, moving, None).unwrap();
    })
    .unwrap();
    yes(
        &mut runtime,
        "window.__scope996__ === undefined && !('__scope996__' in window)",
    );
    yes(
        &mut runtime,
        "document.body.append(namedDiv996); window.__scope996__ === namedDiv996",
    );
}

#[test]
fn document_replacement_never_reuses_equal_generation_cache_entries() {
    use crate::runtime::webidl::with_state;
    let (mut host, _, _, body) = StubHost::page();
    let node = host
        .document
        .append_element(Some(body), "div", None::<&str>);
    host.document
        .set_element_attribute(node, "id", "__replaced996__")
        .unwrap();
    let mut runtime = DomRuntime::new(host).unwrap();
    yes(&mut runtime, "'__replaced996__' in window");
    let generation = with_state(runtime.context_mut(), |s| s.generation).unwrap();
    with_state(runtime.context_mut(), |s| {
        let (mut replacement, _, _, body) = StubHost::page();
        let node = replacement
            .document
            .append_element(Some(body), "div", None::<&str>);
        replacement
            .document
            .set_element_attribute(node, "id", "__newDocument996__")
            .unwrap();
        *s.host.document_mut() = replacement.document;
    })
    .unwrap();
    yes(
        &mut runtime,
        "!('__replaced996__' in window) && ('__newDocument996__' in window)",
    );
    assert_eq!(
        with_state(runtime.context_mut(), |s| s.generation).unwrap(),
        generation
    );
}
