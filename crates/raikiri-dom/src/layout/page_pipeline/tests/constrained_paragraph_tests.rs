use super::*;

#[test]
fn height_constraints_on_ancestors_keep_a_paragraph_in_plain_block_flow() {
    // The own height of the paragraph and the heights of its ancestors may be
    // constrained: moving its lines grows their boxes only as far as that
    // allows. A box that clips at a set height, size containment, floats,
    // absolute positioning and grid and table layout place their content some
    // other way. A flex container whose only item is the paragraph, or holds
    // it, grows around the moved lines like a block.
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
        ("display:flex", "height:100px", true),
        ("display:flex;flex-direction:column", "height:100px", true),
        ("display:flex;height:100px", "height:100px", true),
        (
            "display:flex;overflow:hidden;height:100px",
            "height:100px",
            false,
        ),
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

#[test]
fn a_flex_container_with_another_item_keeps_its_paragraph_out_of_plain_block_flow() {
    // Another item on a flex line sits beside the paragraph, or after it in a
    // column, and does not follow the lines that move down inside it. Boxes
    // that are on no flex line leave the paragraph the only item.
    let cases = [
        ("display:flex", "display:block;height:10px", false),
        (
            "display:flex;flex-direction:column",
            "display:block;height:10px",
            false,
        ),
        (
            "display:flex;flex-wrap:wrap",
            "display:block;height:10px;width:100%",
            false,
        ),
        ("display:flex", "display:contents", false),
        ("display:flex", "display:none", true),
        ("display:flex", "position:absolute;height:10px", true),
        ("display:flex", "position:fixed;height:10px", true),
    ];
    for (ancestor_style, sibling_style, expected) in cases {
        let mut paragraph = 0;
        let mut fixture =
            crate::layout::ifc::test_support::block_fixture(ancestor_style, |doc, wrapper| {
                paragraph = doc.append_element(
                    Some(wrapper),
                    "div",
                    Some("display:block;height:100px".to_owned()),
                );
                doc.append_element(Some(wrapper), "div", Some(sibling_style.to_owned()));
            });
        let body = fixture.doc.parent_of(fixture.root).unwrap();
        let session = super::super::super::context::LayoutSession::bridged(
            &mut fixture.doc,
            &fixture.cascade,
        );
        assert_eq!(
            is_block_flow_paragraph(&session, &fixture.cascade, paragraph, body),
            expected,
            "ancestor={ancestor_style}, sibling={sibling_style}"
        );
    }
}
