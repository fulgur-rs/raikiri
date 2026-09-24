use super::*;
use taffy::Style;

#[test]
fn append_child_moves_elements_and_rejects_cycles() {
    let mut doc = Document::new();
    let first = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let second = doc.append_element(Some(0), "section", Style::default(), None::<&str>);
    let child = doc.append_element(Some(first), "span", Style::default(), None::<&str>);

    doc.append_child(second, child).unwrap();
    assert!(doc.nodes[first].children.is_empty());
    assert_eq!(doc.nodes[second].children, vec![child]);
    assert_eq!(doc.parent_of(child), Some(second));
    assert!(doc.append_child(child, second).is_err());
    assert_eq!(doc.parent_of(child), Some(second));
}

#[test]
fn append_child_rejects_stale_indices_and_non_dom_kinds() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let child = doc.append_element(None, "span", Style::default(), None::<&str>);
    let comment = doc.append_comment(Some(host), "not appendable");

    assert!(doc.append_child(usize::MAX, child).is_err());
    assert!(doc.append_child(0, child).is_err());
    assert!(doc.append_child(host, usize::MAX).is_err());
    assert!(doc.append_child(host, comment).is_err());
}

#[test]
fn append_child_splices_fragment_children() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "main", Style::default(), None::<&str>);
    let fragment = doc.nodes.len();
    doc.nodes.push(Node::new_document_fragment());
    let child = doc.append_text(fragment, "from fragment");

    doc.append_child(host, fragment).unwrap();
    assert_eq!(doc.nodes[host].children, vec![child]);
    assert!(doc.nodes[fragment].children.is_empty());
    assert_eq!(doc.parent_of(child), Some(host));
    assert_eq!(doc.parent_of(fragment), None);
}

#[test]
fn detached_element_creation_validates_xml_names() {
    let mut doc = Document::new();
    let element = doc.create_detached_element("my-widget").unwrap();
    assert_eq!(doc.nodes[element].tag_name(), Some("my-widget"));

    let count = doc.nodes.len();
    assert!(doc.create_detached_element("bad>tag").is_err());
    assert!(doc.create_detached_element("").is_err());
    assert_eq!(doc.nodes.len(), count);
}

#[test]
fn element_text_content_reads_descendants_and_replaces_them() {
    let mut doc = Document::new();
    let host = doc.append_element(Some(0), "div", Style::default(), None::<&str>);
    let nested = doc.append_element(Some(host), "span", Style::default(), None::<&str>);
    let nested_text = doc.append_text(nested, "first");
    doc.append_comment(Some(nested), "ignored comment");
    doc.append_text(host, " second");

    assert_eq!(
        doc.element_text_content(host).as_deref(),
        Some("first second")
    );
    assert_eq!(doc.element_text_content(nested_text), None);
    assert!(doc.set_element_text_content(usize::MAX, "invalid").is_err());
    assert!(
        doc.set_element_text_content(nested_text, "invalid")
            .is_err()
    );
    doc.set_element_text_content(host, "replacement").unwrap();
    assert_eq!(
        doc.element_text_content(host).as_deref(),
        Some("replacement")
    );
    assert_eq!(doc.nodes[host].children.len(), 1);
    assert_eq!(doc.parent_of(nested), None);

    doc.set_element_text_content(host, "").unwrap();
    assert_eq!(doc.element_text_content(host).as_deref(), Some(""));
    assert!(doc.nodes[host].children.is_empty());
}
