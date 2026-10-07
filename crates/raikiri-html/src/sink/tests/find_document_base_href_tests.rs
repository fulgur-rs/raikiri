use super::find_document_base_href;
use raikiri_dom::Document;

#[test]
fn document_without_a_head_element_yields_no_base_href() {
    // Mirrors collect_external_stylesheet_hrefs_tests's identical case:
    // find_head_element's None branch, only reachable via a hand-built
    // Document (html5ever's tree construction always synthesizes a
    // <head>). Full document-order / trim / empty-href / <template>
    // behavior is exercised at the `parse()` level in lib.rs, where a
    // mock `NetworkProvider` can observe which URL was actually
    // resolved and requested.
    let doc = Document::new();
    assert!(find_document_base_href(&doc).is_none());
}
