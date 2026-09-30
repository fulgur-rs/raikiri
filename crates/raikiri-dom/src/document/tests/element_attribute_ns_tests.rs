use super::*;
use taffy::Style;

const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

fn svg_use_element(doc: &mut Document) -> usize {
    let id = doc.append_element(Some(0), "use", Style::default(), None::<&str>);
    doc.set_element_namespace_info(id, Some(SVG_NS.into()), None);
    doc.set_element_namespaced_attribute(id, XLINK_NS, Some("xlink".into()), "href", "#shape")
        .expect("valid namespaced attribute");
    id
}

#[test]
fn namespaced_attribute_survives_and_null_lookup_ignores_it() {
    let mut doc = Document::new();
    let id = svg_use_element(&mut doc);
    // Namespace-qualified lookup finds the value.
    assert_eq!(
        doc.element_attribute_ns(id, XLINK_NS, "href"),
        Some("#shape")
    );
    // Null-namespace lookup does not leak namespaced values.
    assert_eq!(doc.element_attribute(id, "href"), None);
    // Node-level accessors agree.
    let node = doc.get_node(id).expect("element exists");
    assert_eq!(node.attribute_ns(XLINK_NS, "href"), Some("#shape"));
    assert_eq!(node.attribute("href"), None);
}

#[test]
fn namespaced_lookup_is_exact_without_case_folding() {
    let mut doc = Document::new();
    let id = doc.append_element(Some(0), "svg", Style::default(), None::<&str>);
    doc.set_element_namespace_info(id, Some(SVG_NS.into()), None);
    doc.set_element_namespaced_attribute(id, SVG_NS, None, "viewBox", "0 0 1 1")
        .expect("valid namespaced attribute");
    assert_eq!(
        doc.element_attribute_ns(id, SVG_NS, "viewBox"),
        Some("0 0 1 1")
    );
    assert_eq!(doc.element_attribute_ns(id, SVG_NS, "viewbox"), None);
    assert_eq!(doc.element_attribute_ns(id, XLINK_NS, "viewBox"), None);
}

#[test]
fn namespaced_lookup_returns_none_for_non_elements_and_bad_ids() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let text = doc.append_text(host, "hi");
    assert_eq!(doc.element_attribute_ns(text, XLINK_NS, "href"), None);
    assert_eq!(doc.element_attribute_ns(usize::MAX, XLINK_NS, "href"), None);
    let root = doc.root_index();
    let root_node = doc.get_node(root).expect("root exists");
    assert_eq!(root_node.attribute_ns(XLINK_NS, "href"), None);
}

#[test]
fn namespaced_setter_rejects_empty_namespace() {
    let mut doc = Document::new();
    let id = doc.append_element(Some(0), "use", Style::default(), None::<&str>);
    assert!(
        doc.set_element_namespaced_attribute(id, "", Some("xlink".into()), "href", "#a")
            .is_err()
    );
    assert_eq!(doc.element_attribute_ns(id, "", "href"), None);
}
