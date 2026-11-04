use super::*;

#[test]
fn negative_margin_on_an_empty_previous_sibling_keeps_the_first_break_after_content() {
    let mut next = 0;
    let (doc, pages) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:250px"));
        doc.append_element(
            Some(body),
            "div",
            Some("display:block;height:0;margin-top:-100px;margin-bottom:300px"),
        );
        next = doc.append_element(
            Some(body),
            "div",
            Some("display:block;height:20px;margin-top:-200px"),
        );
    });
    // Before pagination, the empty border is at 150 and the next box at 350.
    // Their combined margin starts at 250, so its first page edge is 300.
    assert_eq!(page_flow_y(&doc, next), 300.0);
    assert_eq!(pages, 4);
}
