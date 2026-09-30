use super::super::*;
use taffy::Style;

const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

fn element_ref(doc: &Document, id: usize) -> ElementRef<'_> {
    ElementRef {
        node: &doc.nodes[id],
    }
}

#[test]
fn attr_ns_reads_xlink_href_while_attr_does_not() {
    let mut doc = Document::new();
    let id = doc.append_element(Some(0), "use", Style::default(), None::<&str>);
    doc.set_element_namespace_info(id, Some(SVG_NS.into()), None);
    doc.set_element_namespaced_attribute(id, XLINK_NS, Some("xlink".into()), "href", "#shape")
        .expect("valid namespaced attribute");
    let element = element_ref(&doc, id);
    assert_eq!(
        raikiri_traits::Element::attr_ns(&element, XLINK_NS, "href"),
        Some("#shape")
    );
    assert_eq!(raikiri_traits::Element::attr(&element, "href"), None);
}

#[test]
fn attr_ns_is_exact_and_returns_none_when_absent() {
    let mut doc = Document::new();
    let id = doc.append_element(Some(0), "use", Style::default(), None::<&str>);
    doc.set_element_namespaced_attribute(id, XLINK_NS, Some("xlink".into()), "href", "#a")
        .expect("valid namespaced attribute");
    let element = element_ref(&doc, id);
    assert_eq!(
        raikiri_traits::Element::attr_ns(&element, XLINK_NS, "HREF"),
        None
    );
    assert_eq!(
        raikiri_traits::Element::attr_ns(&element, SVG_NS, "href"),
        None
    );
    // Null-namespace storage never leaks into the namespaced view.
    doc.set_element_attribute(id, "href", "#plain")
        .expect("valid attribute");
    let element = element_ref(&doc, id);
    assert_eq!(
        raikiri_traits::Element::attr_ns(&element, XLINK_NS, "href"),
        Some("#a")
    );
    assert_eq!(
        raikiri_traits::Element::attr(&element, "href"),
        Some("#plain")
    );
}

#[test]
fn attr_ns_reaches_through_dom_trait_object() {
    let mut doc = Document::new();
    let id = doc.append_element(Some(0), "use", Style::default(), None::<&str>);
    doc.set_element_namespaced_attribute(id, XLINK_NS, Some("xlink".into()), "href", "#shape")
        .expect("valid namespaced attribute");
    let node_id = raikiri_traits::NodeId::new(id as u64);
    let node = <Document as raikiri_traits::Dom>::node(&doc, node_id).expect("node exists");
    let element = node.as_element().expect("use is element");
    assert_eq!(element.attr_ns(XLINK_NS, "href"), Some("#shape"));
}
