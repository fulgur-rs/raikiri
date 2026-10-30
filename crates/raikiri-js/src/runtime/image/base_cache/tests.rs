use super::*;

fn document(href: &str) -> Document {
    let mut d = Document::new();
    let base = d.append_element(Some(0), "base", None::<&str>);
    d.set_element_namespace(base, Some("http://www.w3.org/1999/xhtml".into()));
    d.set_element_attribute(base, "href", href).unwrap();
    d
}
#[test]
fn absent_results_and_only_the_latest_host_key_are_retained() {
    let d = Document::new();
    let mut cache = BaseUrlCache::default();
    assert_eq!(cache.resolve(&d, None, "relative.png"), None);
    assert!(cache.entry.as_ref().unwrap().base.is_none());
    assert_eq!(
        cache.resolve(&d, None, "https://image.test/x"),
        Some("https://image.test/x".into())
    );
    for index in 0..32 {
        let host = format!("https://host{index}.test/dir/page");
        assert_eq!(
            cache.resolve(&d, Some(host.clone()), "a.png"),
            Some(format!("https://host{index}.test/dir/a.png"))
        );
        assert_eq!(
            cache.entry.as_ref().unwrap().host_url.as_deref(),
            Some(host.as_str())
        );
    }
    assert_eq!(
        cache.resolve(&d, Some("invalid".into()), "relative.png"),
        None
    );
    assert!(cache.entry.as_ref().unwrap().base.is_none());
}
#[test]
fn document_replacement_clone_and_extraction_refresh_the_snapshot() {
    let first = document("https://first.test/");
    let second = document("https://second.test/");
    let mut cache = BaseUrlCache::default();
    let host = Some("https://host.test/page".into());
    assert_eq!(
        cache.resolve(&first, host.clone(), "a.png"),
        Some("https://first.test/a.png".into())
    );
    assert_eq!(
        cache.resolve(&second, host.clone(), "a.png"),
        Some("https://second.test/a.png".into())
    );
    let cloned = second.clone();
    cache.resolve(&cloned, host.clone(), "a.png");
    assert_eq!(
        &cache.entry.as_ref().unwrap().version,
        cloned.base_url_version()
    );
    let extracted = second.extract_nodes(&(0..second.node_count()).collect::<Vec<_>>());
    cache.resolve(&extracted, host, "a.png");
    assert_eq!(
        &cache.entry.as_ref().unwrap().version,
        extracted.base_url_version()
    );
}
