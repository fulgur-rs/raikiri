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

/// A box that avoids a break inside it, the first in the document below a
/// block whose margin collapses through `body`, with `wrapper_css` on that
/// block. Returns the box's top and the page count.
fn avoided_box_below_a_collapsed_margin(wrapper_css: &str) -> (f32, usize) {
    let mut avoided = 0;
    let (doc, pages) = paged_body(|doc, body| {
        let wrapper = doc.append_element(Some(body), "div", Some(wrapper_css));
        avoided = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;padding-bottom:40px;break-inside:avoid"),
        );
        for _ in 0..2 {
            doc.append_element(Some(avoided), "div", Some("display:block;height:30px"));
        }
        doc.append_element(Some(body), "div", Some("display:block;height:10px"));
    });
    (page_flow_y(&doc, avoided), pages)
}

#[test]
fn a_margin_that_collapses_through_body_starts_the_page_for_an_avoided_box() {
    // The margin of the wrapper collapses through `body`, so the margin box of
    // the wrapper, and the avoided box with it, starts the first page. The box
    // stays there and breaks inside, as in Chromium.
    for wrapper_css in [
        "display:block;margin-top:5px",
        "display:block;margin-top:5px;padding-top:5px",
        "display:block;margin-top:5px;border-top:5px solid",
        "display:flow-root;margin-top:5px",
    ] {
        let (top, pages) = avoided_box_below_a_collapsed_margin(wrapper_css);
        assert!(
            top < 100.0,
            "{wrapper_css}: the box stays on the first page at {top}"
        );
        assert_eq!(pages, 2, "{wrapper_css}");
    }
    // Content above the wrapper leaves room to move into: the box moves
    // whole, with the wrapper's margin truncated at the break.
    let mut avoided = 0;
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:5px"));
        let wrapper = doc.append_element(Some(body), "div", Some("display:block;margin-top:5px"));
        avoided = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;padding-bottom:40px;break-inside:avoid"),
        );
        for _ in 0..2 {
            doc.append_element(Some(avoided), "div", Some("display:block;height:30px"));
        }
    });
    assert_eq!(page_flow_y(&doc, avoided), 100.0);
}
