use super::*;

#[cfg(feature = "image-base-diagnostics")]
#[test]
fn repeated_image_sources_share_absent_base_discovery() {
    for absolute in [false, true] {
        let (mut host, body) = host();
        for _ in 0..128 {
            host.document
                .append_element(Some(body), "span", None::<&str>);
        }
        let nodes = host.document.node_count() as u64;
        let mut rt = DomRuntime::new(host).unwrap();
        raikiri_dom::begin_base_url_diagnostics();
        for _ in 0..16 {
            let result = super::super::resolve_source(
                rt.context_mut(),
                if absolute { LOADS } else { "ok.png" },
            )
            .unwrap();
            assert_eq!(result.as_deref(), Some(LOADS));
        }
        let work = raikiri_dom::finish_base_url_diagnostics();
        assert_eq!(work.searches, 1);
        assert!(work.node_visits <= nodes);
    }
}

fn oracle(document: &raikiri_dom::Document, host_url: Option<&str>, src: &str) -> Option<String> {
    let base = match host_url.and_then(|text| url::Url::parse(text).ok()) {
        Some(url) => document.live_base_url(Some(&url)),
        None => document.base_url().cloned(),
    };
    url::Url::options()
        .base_url(base.as_ref())
        .parse(src)
        .ok()
        .map(String::from)
}

#[test]
fn image_source_parser_keeps_base_dependent_and_fallback_semantics() {
    for host_url in [
        Some(PAGE),
        Some("file:///C:/dir/page.html"),
        Some("data:text/plain,opaque"),
        Some("not a URL"),
        None,
    ] {
        for href in [
            None,
            Some("assets/"),
            Some("http://["),
            Some("data:text/plain,x"),
            Some("javascript:void(0)"),
        ] {
            let (mut host, _, head, _) = StubHost::page();
            host.document_url = host_url.map(str::to_owned);
            host.document.set_base_url(Some(
                url::Url::parse("https://recorded.test/fallback/").unwrap(),
            ));
            if let Some(href) = href {
                let base = host
                    .document
                    .append_element(Some(head), "base", None::<&str>);
                host.document
                    .set_element_namespace(base, Some("http://www.w3.org/1999/xhtml".into()));
                host.document
                    .set_element_attribute(base, "href", href)
                    .unwrap();
            }
            let sources = [
                LOADS,
                "ok.png",
                "https:foo",
                "https:/foo",
                "file:foo",
                "file:/foo",
                "file:///foo",
                "#fragment",
                "?query",
                "",
                " ",
                "data:image/png,abc",
                "http://[",
                "//other.test/x",
            ];
            let expected = sources.map(|src| oracle(&host.document, host_url, src));
            let mut rt = DomRuntime::new(host).unwrap();
            for (src, expected) in sources.into_iter().zip(expected) {
                assert_eq!(
                    super::super::resolve_source(rt.context_mut(), src).unwrap(),
                    expected
                );
            }
        }
    }
}

#[cfg(feature = "image-base-diagnostics")]
#[test]
fn detached_new_images_and_src_updates_do_not_invalidate_base_discovery() {
    let mut rt = rt();
    let nodes = with_state(rt.context_mut(), |s| s.host.document().node_count()).unwrap();
    raikiri_dom::begin_base_url_diagnostics();
    ok(
        &mut rt,
        "(()=>{for(let i=0;i<32;++i){let img=new Image();img.src='ok.png';if(img.src!=='https://example.test/dir/ok.png')return false;}return true;})()",
    );
    let work = raikiri_dom::finish_base_url_diagnostics();
    assert_eq!(work.searches, 1);
    assert!(work.node_visits <= nodes as u64);
}

fn resolve(rt: &mut DomRuntime) -> String {
    super::super::resolve_source(rt.context_mut(), "ok.png")
        .unwrap()
        .unwrap()
}

#[test]
fn native_base_mutations_refresh_without_runtime_generation_changes() {
    let (mut host, _, head, _) = StubHost::page();
    host.document_url = Some(PAGE.into());
    let mut rt = DomRuntime::new(host).unwrap();
    let generation = with_state(rt.context_mut(), |s| s.generation).unwrap();
    assert_eq!(resolve(&mut rt), LOADS);
    let (first, second) = with_state(rt.context_mut(), |s| {
        let d = s.host.document_mut();
        let first = d.append_element(Some(head), "base", None::<&str>);
        d.set_element_namespace(first, Some("http://www.w3.org/1999/xhtml".into()));
        d.set_element_attribute(first, "href", "one/").unwrap();
        let second = d.append_element(Some(head), "base", None::<&str>);
        d.set_element_namespace(second, Some("http://www.w3.org/1999/xhtml".into()));
        d.set_element_attribute(second, "href", "two/").unwrap();
        (first, second)
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), "https://example.test/dir/one/ok.png");
    with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .pre_insert(head, second, Some(first))
            .unwrap()
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), "https://example.test/dir/two/ok.png");
    with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .set_element_attribute(second, "href", "http://[")
            .unwrap()
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), LOADS);
    with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .set_element_namespace(second, Some("http://www.w3.org/2000/svg".into()))
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), "https://example.test/dir/one/ok.png");
    with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .set_element_namespace(second, Some("http://www.w3.org/1999/xhtml".into()))
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), LOADS);
    with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .set_element_attributes(second, vec![("href".into(), "three/".into())])
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), "https://example.test/dir/three/ok.png");
    with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .remove_element_attribute_ns(second, None, "href")
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), "https://example.test/dir/one/ok.png");
    with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .set_element_attribute_exact(second, "href", "four/")
            .unwrap()
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), "https://example.test/dir/four/ok.png");
    with_state(rt.context_mut(), |s| {
        s.host
            .document_mut()
            .remove_element_attribute(second, "href")
            .unwrap()
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), "https://example.test/dir/one/ok.png");
    with_state(rt.context_mut(), |s| {
        s.host.document_mut().detach_from_parent(first)
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), LOADS);
    with_state(rt.context_mut(), |s| {
        s.host.document_mut().append_child(head, first).unwrap()
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), "https://example.test/dir/one/ok.png");
    assert_eq!(
        with_state(rt.context_mut(), |s| s.generation).unwrap(),
        generation
    );
}

#[test]
fn host_url_and_recorded_fallback_changes_refresh_the_latest_entry() {
    let mut rt = rt();
    assert_eq!(resolve(&mut rt), LOADS);
    with_state(rt.context_mut(), |s| {
        let host = s.host.as_any_mut().downcast_mut::<StubHost>().unwrap();
        host.document_url = Some("https://other.test/new/page".into());
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), "https://other.test/new/ok.png");
    for host_url in [Some("invalid"), None] {
        with_state(rt.context_mut(), |s| {
            let host = s.host.as_any_mut().downcast_mut::<StubHost>().unwrap();
            host.document_url = host_url.map(str::to_owned);
            host.document
                .set_base_url(Some(url::Url::parse("https://recorded.test/one/").unwrap()));
        })
        .unwrap();
        assert_eq!(resolve(&mut rt), "https://recorded.test/one/ok.png");
        with_state(rt.context_mut(), |s| {
            s.host
                .document_mut()
                .set_base_url(Some(url::Url::parse("https://recorded.test/two/").unwrap()))
        })
        .unwrap();
        assert_eq!(resolve(&mut rt), "https://recorded.test/two/ok.png");
    }
    with_state(rt.context_mut(), |s| {
        s.host
            .as_any_mut()
            .downcast_mut::<StubHost>()
            .unwrap()
            .document_url = Some(PAGE.into())
    })
    .unwrap();
    assert_eq!(resolve(&mut rt), LOADS);
}
