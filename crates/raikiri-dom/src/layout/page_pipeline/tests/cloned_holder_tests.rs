use super::*;
use crate::layout::test_support::page_box_800x600;

#[test]
fn a_slice_edge_cannot_be_hidden_by_an_offsetting_negative_margin() {
    let mut child = 0;
    let mut fixture = crate::layout::ifc::test_support::block_fixture_with_ahem(
        "height:400px;border-top:20px solid;box-decoration-break:clone",
        |doc, root| {
            let middle =
                doc.append_element(Some(root), "div", Some("display:block;padding-top:10px"));
            child = doc.append_element(
                Some(middle),
                "div",
                Some("display:block;contain:size;height:450px;margin-top:-10px"),
            );
        },
    );
    layout_single_page(&mut fixture.doc, &fixture.cascade, page_box_800x600()).expect("layout");
    let body = fixture.doc.parent_of(fixture.root).expect("body");
    let session =
        crate::layout::context::LayoutSession::resumed(&mut fixture.doc, &fixture.cascade);
    let lead = lead_below_ancestor(&session, &fixture.cascade, body, child, fixture.root);
    // The aggregate matches the cloned edge, but includes a slice edge and
    // a negative margin. Equality alone does not prove the lead's ownership.
    assert_eq!(lead, 20.0);
    assert!(!contained_cloned_leading_holder(
        &session,
        &fixture.cascade,
        child,
        fixture.root,
        lead,
    ));
}
