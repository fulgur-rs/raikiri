use super::*;
use crate::runtime::stylesheet::tests::take_list_work;

#[test]
fn cached_empty_and_nonempty_results_walk_once_per_native_revision() {
    let mut document = Document::new();
    let mut cache = SheetList::default();
    take_list_work();
    assert!(cache.nodes(&document).is_empty());
    assert!(cache.nodes(&document).is_empty());
    assert_eq!(take_list_work()[0], 1);
    let style = document.append_element(Some(0), "style", None::<&str>);
    assert_eq!(cache.nodes(&document), [style]);
    assert_eq!(cache.nodes(&document), [style]);
    assert_eq!(take_list_work()[0], 1);
    document.detach_from_parent(style);
    assert!(cache.nodes(&document).is_empty());
    assert!(cache.nodes(&document).is_empty());
    assert_eq!(take_list_work()[0], 1);
    assert_eq!(cache.nodes.capacity(), 0);
}

#[test]
fn document_clone_identity_refreshes_the_list_and_preserves_tree_order() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", None::<&str>);
    let one = document.append_element(Some(html), "style", None::<&str>);
    let two = document.append_element(Some(html), "style", None::<&str>);
    let mut cache = SheetList::default();
    assert_eq!(cache.nodes(&document), [one, two]);
    let clone = document.clone();
    take_list_work();
    assert_eq!(cache.nodes(&clone), [one, two]);
    assert_eq!(take_list_work()[0], 1);
    document.pre_insert(html, two, Some(one)).unwrap();
    assert_eq!(cache.nodes(&document), [two, one]);
}

#[test]
fn nested_style_traversal_matches_the_existing_namespace_and_type_rules() {
    let mut document = Document::new();
    let outer = document.append_element(Some(0), "style", None::<&str>);
    let inner = document.append_element(Some(outer), "style", None::<&str>);
    let mut cache = SheetList::default();
    assert_eq!(cache.nodes(&document), [outer]);
    document
        .set_element_attribute(outer, "type", "text/plain")
        .unwrap();
    assert!(cache.nodes(&document).is_empty());
    document.set_element_namespace(outer, Some("urn:foreign".into()));
    assert_eq!(cache.nodes(&document), [inner]);
}
