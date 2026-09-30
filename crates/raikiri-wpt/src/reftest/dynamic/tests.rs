use super::*;

#[test]
fn waiting_returns_false_without_html_element() {
    let document = raikiri_dom::Document::new();
    assert!(
        !waiting(&document),
        "a fresh Document has no html element, so no reftest-wait can be present"
    );
}
