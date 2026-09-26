use super::super::*;
use taffy::Style;

/// Builds a `Document` with a single `<div>` element (child of root)
/// carrying one attribute, and returns the element's arena id.
fn doc_with_attr(local: &str, value: &str) -> (Document, usize) {
    let mut doc = Document::new();
    let root = doc.root_index();
    let id = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
    doc.set_element_attributes(id, vec![(local.into(), value.into())]);
    (doc, id)
}

fn element_ref(doc: &Document, id: usize) -> ElementRef<'_> {
    ElementRef {
        node: &doc.nodes[id],
    }
}

// Exercise both trait families' `attr()` explicitly (not just the shared
// inherent method) so a missing/incorrect override in either `impl`
// block would be caught, not masked by inherent-method dot-call
// resolution.

#[test]
fn attr_present_with_empty_value_returns_some_empty_string() {
    let (doc, id) = doc_with_attr("data-x", "");
    let er = element_ref(&doc, id);
    assert_eq!(raikiri_traits::Element::attr(&er, "data-x"), Some(""));
    assert_eq!(StyleElement::attr(&er, "data-x"), Some(""));
}

#[test]
fn attr_absent_returns_none() {
    let (doc, id) = doc_with_attr("data-x", "");
    let er = element_ref(&doc, id);
    assert_eq!(raikiri_traits::Element::attr(&er, "data-y"), None);
    assert_eq!(StyleElement::attr(&er, "data-y"), None);
}

#[test]
fn attr_present_with_nonempty_value_returns_value() {
    let (doc, id) = doc_with_attr("data-x", "foo");
    let er = element_ref(&doc, id);
    assert_eq!(raikiri_traits::Element::attr(&er, "data-x"), Some("foo"));
    assert_eq!(StyleElement::attr(&er, "data-x"), Some("foo"));
}

#[test]
fn id_empty_value_normalizes_to_none() {
    let (doc, id) = doc_with_attr("id", "");
    let er = element_ref(&doc, id);
    assert_eq!(raikiri_traits::Element::id(&er), None);
    assert_eq!(StyleElement::id(&er), None);
    // attr("id") itself still preserves presence (Some("")); only id()
    // applies the empty-is-absent normalization.
    assert_eq!(raikiri_traits::Element::attr(&er, "id"), Some(""));
    assert_eq!(StyleElement::attr(&er, "id"), Some(""));
}

#[test]
fn id_present_nonempty_returns_value() {
    let (doc, id) = doc_with_attr("id", "main");
    let er = element_ref(&doc, id);
    assert_eq!(raikiri_traits::Element::id(&er), Some("main"));
    assert_eq!(StyleElement::id(&er), Some("main"));
}

#[test]
fn id_absent_returns_none() {
    let (doc, id) = doc_with_attr("data-x", "y"); // no `id` attribute set
    let er = element_ref(&doc, id);
    assert_eq!(raikiri_traits::Element::id(&er), None);
    assert_eq!(StyleElement::id(&er), None);
}
