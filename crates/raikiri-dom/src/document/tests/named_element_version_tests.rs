use super::*;

fn changes(document: &mut Document, mutate: impl FnOnce(&mut Document)) {
    let before = document.named_element_version().clone();
    mutate(document);
    assert_ne!(&before, document.named_element_version());
}

#[test]
fn named_element_version_tracks_native_attributes_namespaces_and_topology() {
    let mut document = Document::new();
    let parent = document.append_element(Some(0), "section", None::<&str>);
    let node = document.append_element(Some(parent), "div", None::<&str>);
    changes(&mut document, |doc| {
        doc.set_element_attribute(node, "ID", "one").unwrap();
    });
    changes(&mut document, |doc| {
        doc.set_element_attribute_exact(node, "id", "two").unwrap();
    });
    changes(&mut document, |doc| {
        doc.set_element_attribute(node, "name", "three").unwrap();
    });
    changes(&mut document, |doc| {
        doc.remove_element_attribute(node, "NAME").unwrap();
    });
    changes(&mut document, |doc| {
        doc.remove_element_attribute_ns(node, None, "id");
    });
    changes(&mut document, |doc| {
        doc.set_element_attributes(node, vec![("id".into(), "bulk".into())]);
    });
    changes(&mut document, |doc| {
        doc.set_element_namespace(node, Some("http://www.w3.org/2000/svg".into()));
    });
    changes(&mut document, |doc| {
        doc.set_element_namespace_info(node, None, None);
    });
    changes(&mut document, |doc| {
        doc.detach_from_parent(node);
    });
    changes(&mut document, |doc| {
        doc.attach_child(parent, node);
    });
    assert!(document.take_tree_changes().is_empty());
    document.enable_tree_change_tracking();
    let before = document.named_element_version().clone();
    document
        .set_element_attribute(node, "id", "tracked")
        .unwrap();
    assert_ne!(&before, document.named_element_version());
    assert!(document.take_tree_changes().is_empty());
}

#[test]
fn cloned_and_extracted_documents_have_distinct_named_element_versions() {
    let mut document = Document::new();
    let node = document.append_element(Some(0), "div", None::<&str>);
    let snapshot = document.named_element_version().clone();
    assert_eq!(&snapshot, document.named_element_version());
    let restored = Document::from_logical_snapshot(document.logical_snapshot(), 100).unwrap();
    assert_ne!(&snapshot, restored.named_element_version());
    assert_ne!(&snapshot, document.clone().named_element_version());
    assert_ne!(
        &snapshot,
        document.extract_nodes(&[0, node]).named_element_version()
    );
    assert_ne!(
        Document::new().named_element_version(),
        Document::new().named_element_version()
    );
}

#[test]
fn irrelevant_attributes_do_not_change_named_element_version() {
    let mut document = Document::new();
    let node = document.append_element(Some(0), "div", None::<&str>);
    let snapshot = document.named_element_version().clone();
    document
        .set_element_attribute(node, "class", "changed")
        .unwrap();
    document
        .set_element_namespaced_attribute(node, "urn:example", None, "id", "foreign")
        .unwrap();
    document.remove_element_attribute_ns(node, Some("urn:example"), "id");
    assert_eq!(&snapshot, document.named_element_version());
}
