use super::*;

#[test]
fn own_height_constraints_do_not_relax_plain_flow_ancestor_boundaries() {
    let cases = [
        ("", "height:100px", true),
        ("", "min-height:125px", true),
        ("", "max-height:90px", true),
        ("", "height:100px;contain:size", false),
        ("height:100px", "height:100px", false),
        ("min-height:100px", "height:100px", false),
        ("max-height:100px", "height:100px", false),
        ("contain:size", "height:100px", false),
        ("float:left", "height:100px", false),
        ("position:absolute", "height:100px", false),
        ("display:flex", "height:100px", false),
        ("display:grid", "height:100px", false),
        ("display:table", "height:100px", false),
    ];
    for (ancestor_style, own_style, expected) in cases {
        let mut paragraph = 0;
        let mut fixture =
            crate::layout::ifc::test_support::block_fixture(ancestor_style, |doc, wrapper| {
                paragraph = doc.append_element(
                    Some(wrapper),
                    "div",
                    Some(format!("display:block;{own_style}")),
                );
            });
        let body = fixture.doc.parent_of(fixture.root).unwrap();
        let session = super::super::super::context::LayoutSession::bridged(
            &mut fixture.doc,
            &fixture.cascade,
        );
        assert_eq!(
            is_block_flow_paragraph(&session, &fixture.cascade, paragraph, body),
            expected,
            "ancestor={ancestor_style}, paragraph={own_style}"
        );
    }
}
