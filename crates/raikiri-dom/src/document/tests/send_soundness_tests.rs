use super::*;
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;

#[test]
fn a_laid_out_document_with_calc_payloads_moves_across_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<Document>();

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block;width:100px"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    let child = doc.append_element(
        Some(body),
        "div",
        Some("display:block;width:calc(50% + 10px);height:10px;max-width:calc(10% + 80px)"),
    );
    doc.mark_in_document_flags();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;

    let rules = build_rule_tree(&doc);
    let layout_cascade = cascade(&doc, &rules).expect("cascade Ok");
    crate::layout::layout_single_page(&mut doc, &layout_cascade, page).expect("layout Ok");
    assert_eq!(doc.nodes[child].layout.size.width, 60.0);

    // The payloads the first layout published travel with the document.
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let rules = build_rule_tree(&doc);
                let moved_cascade = cascade(&doc, &rules).expect("cascade Ok");
                crate::layout::layout_single_page(&mut doc, &moved_cascade, page)
                    .expect("relayout Ok");
                assert_eq!(doc.nodes[child].layout.size.width, 60.0);
            })
            .join()
            .expect("document moves across threads");
    });
}
