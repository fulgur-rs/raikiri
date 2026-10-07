use super::collect_external_stylesheet_hrefs;
use raikiri_dom::Document;

#[test]
fn document_without_a_head_element_yields_no_hrefs() {
    // find_head_element's None branch: a Document that never got a
    // <head> attached at all (html5ever's tree construction always
    // synthesizes one, so this only happens for a hand-built Document
    // like this one — exercised directly since collect_external_stylesheet_hrefs
    // is pub(crate) and doesn't need the full parse pipeline).
    let doc = Document::new();
    assert!(collect_external_stylesheet_hrefs(&doc).is_empty());
}
