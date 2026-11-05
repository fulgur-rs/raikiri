use super::*;

#[test]
fn height_constraints_on_ancestors_keep_a_paragraph_in_plain_block_flow() {
    // The own height of the paragraph and the heights of its ancestors may be
    // constrained: moving its lines grows their boxes only as far as that
    // allows. A box that clips at a set height, size containment, floats,
    // absolute positioning and flex, grid and table layout place their
    // content some other way.
    let cases = [
        ("", "height:100px", true),
        ("", "min-height:125px", true),
        ("", "max-height:90px", true),
        ("", "height:100px;contain:size", false),
        ("height:100px", "height:100px", true),
        ("min-height:100px", "height:100px", true),
        ("max-height:100px", "height:100px", true),
        ("height:100px;overflow:hidden", "height:100px", false),
        ("max-height:100px;overflow:clip", "height:100px", false),
        ("overflow:hidden", "height:100px", true),
        ("height:100px;overflow-x:hidden", "height:100px", false),
        ("contain:size", "height:100px", false),
        ("height:100px;contain:size", "height:100px", false),
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
