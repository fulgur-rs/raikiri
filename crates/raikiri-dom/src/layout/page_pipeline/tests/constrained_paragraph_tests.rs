use super::*;

#[test]
fn height_constraints_on_ancestors_keep_a_paragraph_in_plain_block_flow() {
    // The own height of the paragraph and the heights of its ancestors may be
    // constrained: moving its lines grows their boxes only as far as that
    // allows, and a box that clips at a set height hides the lines moved
    // past its edge, as in Chromium. Size containment, floats, absolute
    // positioning and table layout place their content some other way. A
    // flex container or a grid whose only item is the paragraph, or holds
    // it, grows around the moved lines like a block.
    let cases = [
        ("", "height:100px", true),
        ("", "min-height:125px", true),
        ("", "max-height:90px", true),
        ("", "height:100px;contain:size", false),
        ("height:100px", "height:100px", true),
        ("min-height:100px", "height:100px", true),
        ("max-height:100px", "height:100px", true),
        ("height:100px;overflow:hidden", "height:100px", true),
        ("max-height:100px;overflow:clip", "height:100px", true),
        ("overflow:hidden", "height:100px", true),
        ("height:100px;overflow-x:hidden", "height:100px", true),
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
            true,
        ),
        ("display:grid", "height:100px", true),
        (
            "display:grid;overflow:hidden;height:100px",
            "height:100px",
            true,
        ),
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

/// Whether a 100px paragraph is in plain block flow when it is an item of a
/// container styled `container_style` beside a sibling built by `sibling`.
fn paragraph_item_in_block_flow(
    container_style: &str,
    sibling: impl FnOnce(&mut crate::Document, usize),
) -> bool {
    let mut paragraph = 0;
    let mut fixture =
        crate::layout::ifc::test_support::block_fixture(container_style, |doc, wrapper| {
            paragraph = doc.append_element(
                Some(wrapper),
                "div",
                Some("display:block;height:100px".to_owned()),
            );
            sibling(doc, wrapper);
        });
    let body = fixture.doc.parent_of(fixture.root).unwrap();
    let session =
        super::super::super::context::LayoutSession::bridged(&mut fixture.doc, &fixture.cascade);
    is_block_flow_paragraph(&session, &fixture.cascade, paragraph, body)
}

#[test]
fn a_flex_or_grid_item_keeps_its_paragraph_in_plain_block_flow_beside_other_items() {
    // The container grows around the item. The items beside it keep their
    // places, those on later rows or lines follow the end of its row, and a
    // row of a size of its own holds the moved lines.
    let containers = [
        "display:grid",
        "display:grid;grid-template-rows:60px",
        "display:grid;grid-template-rows:auto",
        "display:grid;grid-template-columns:1fr 1fr",
        "display:grid;height:150px",
        "display:grid;overflow:hidden;height:100px",
        "display:flex",
        "display:flex;flex-direction:column",
        "display:flex;flex-wrap:wrap",
    ];
    let siblings = [
        "display:block;height:10px",
        "display:block;height:10px;width:100%",
        "display:contents",
        "display:none",
        "position:absolute;height:10px",
        "position:fixed;height:10px",
    ];
    for container in containers {
        for sibling in siblings {
            assert!(
                paragraph_item_in_block_flow(container, |doc, wrapper| {
                    doc.append_element(Some(wrapper), "div", Some(sibling.to_owned()));
                }),
                "container={container}, sibling={sibling}"
            );
        }
        for text in [" \t\n\r\u{000c}", "\u{00a0}", "x"] {
            assert!(
                paragraph_item_in_block_flow(container, |doc, wrapper| {
                    doc.append_text(wrapper, text);
                }),
                "container={container}, text={text:?}"
            );
        }
    }
}
