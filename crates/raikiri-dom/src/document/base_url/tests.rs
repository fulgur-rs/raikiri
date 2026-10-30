use super::*;
fn base() -> (Document, usize) {
    let mut d = Document::new();
    let id = d.append_element(Some(0), "base", None::<&str>);
    d.set_element_namespace(id, Some("http://www.w3.org/1999/xhtml".into()));
    (d, id)
}
#[test]
fn native_base_inputs_advance_and_unrelated_image_inputs_do_not() {
    let (mut d, id) = base();
    let mut version = d.base_url_version().clone();
    d.set_element_attribute(id, "HREF", "one/").unwrap();
    assert_ne!(version, *d.base_url_version());
    version = d.base_url_version().clone();
    d.set_element_attribute_exact(id, "href", "two/").unwrap();
    assert_ne!(version, *d.base_url_version());
    version = d.base_url_version().clone();
    d.remove_element_attribute_ns(id, None, "href").unwrap();
    assert_ne!(version, *d.base_url_version());
    version = d.base_url_version().clone();
    d.set_element_attributes(id, vec![("href".into(), "three/".into())]);
    assert_ne!(version, *d.base_url_version());
    version = d.base_url_version().clone();
    d.remove_element_attribute(id, "href").unwrap();
    assert_ne!(version, *d.base_url_version());
    version = d.base_url_version().clone();
    d.set_element_namespace(id, Some("http://www.w3.org/2000/svg".into()));
    assert_ne!(version, *d.base_url_version());
    version = d.base_url_version().clone();
    d.set_base_url(Some(url::Url::parse("https://recorded.test/").unwrap()));
    assert_ne!(version, *d.base_url_version());
    version = d.base_url_version().clone();
    d.set_element_namespaced_attribute(id, "urn:test", Some("test".into()), "href", "ignored/")
        .unwrap();
    d.set_element_attribute_exact(id, "HREF", "also-ignored/")
        .unwrap();
    d.remove_element_attribute_ns(id, None, "HREF").unwrap();
    assert_eq!(version, *d.base_url_version());
    let image = d.create_detached_element("img").unwrap();
    d.set_element_namespace(image, Some("http://www.w3.org/1999/xhtml".into()));
    d.set_element_attributes(image, vec![("src".into(), "image.png".into())]);
    d.set_element_attribute(image, "src", "next.png").unwrap();
    d.set_element_attribute(image, "href", "irrelevant")
        .unwrap();
    d.select_image_url(
        image,
        "next.png",
        url::Url::parse("https://recorded.test/next.png").unwrap(),
    );
    assert_eq!(version, *d.base_url_version());
    d.append_child(0, image).unwrap_err();
    assert_eq!(version, *d.base_url_version());
    d.detach_from_parent(id);
    assert_ne!(version, *d.base_url_version());
}
#[test]
fn clone_extract_and_overflow_never_reuse_old_identity() {
    let (mut d, id) = base();
    let old = d.base_url_version().clone();
    assert_ne!(old, *d.clone().base_url_version());
    let extracted = d.extract_nodes(&[0, id]);
    assert_ne!(old, *extracted.base_url_version());
    d.base_url_inputs.version.generation = u64::MAX;
    let before = d.base_url_version().clone();
    d.note_base_url_change();
    assert_ne!(before, *d.base_url_version());
    assert_eq!(d.base_url_inputs.version.generation, 0);
}

#[test]
fn native_discovery_diagnostics_count_the_actual_preorder_walk() {
    let mut d = Document::new();
    let root = d.append_element(Some(0), "div", None::<&str>);
    for _ in 0..8 {
        d.append_element(Some(root), "span", None::<&str>);
    }
    let host = url::Url::parse("https://host.test/page").unwrap();
    crate::base_url_diagnostics::begin_base_url_diagnostics();
    assert_eq!(d.live_base_url(Some(&host)), Some(host.clone()));
    let work = crate::base_url_diagnostics::finish_base_url_diagnostics();
    assert_eq!(work.searches, 1);
    assert_eq!(work.node_visits, d.node_count() as u64);
}
