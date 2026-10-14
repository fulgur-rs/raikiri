use super::*;

#[test]
fn sideways_ifc_fallback_uses_horizontal_percentage_edge_basis() {
    for mode in [
        raikiri_style::property::WritingMode::SidewaysRl,
        raikiri_style::property::WritingMode::SidewaysLr,
    ] {
        let mut tree = Document::new();
        let parent = tree.append_element(Some(0), "div", Style::default(), None::<&str>);
        let node = tree.append_element(Some(parent), "div", Style::default(), None::<&str>);
        tree.nodes[parent].authored_writing_mode = Some(mode);

        assert_eq!(
            ifc_parent_inline_size(
                &tree,
                node,
                Size {
                    width: Some(60.0),
                    height: None,
                },
            ),
            Some(60.0),
            "{mode:?} uses the horizontal IFC fallback basis"
        );
    }
}

#[test]
fn whitespace_only_text_is_not_a_child_of_a_block_container() {
    // CSS 2.1 9.2.1.1: a whitespace-only run between blocks generates no
    // anonymous block box, so taffy's block algorithm never sees it and it
    // cannot separate the margins of its siblings.
    let mut tree = Document::new();
    let parent = tree.append_element(Some(0), "div", Style::default(), None::<&str>);
    tree.nodes[parent].style.display = Display::Block;
    tree.append_text(parent, "\n  ");
    let first = tree.append_element(Some(parent), "div", Style::default(), None::<&str>);
    tree.append_text(parent, "\t\r\u{000c} ");
    let text = tree.append_text(parent, " x ");
    let last = tree.append_element(Some(parent), "div", Style::default(), None::<&str>);
    tree.append_text(parent, "\n");
    let parent = NodeId::from(parent);

    let children: Vec<usize> = tree.child_ids(parent).map(usize::from).collect();
    assert_eq!(children, vec![first, text, last]);
    assert_eq!(tree.child_count(parent), 3);
    assert_eq!(usize::from(tree.get_child_id(parent, 1)), text);
}

#[test]
fn a_percentage_max_height_of_an_indefinite_basis_does_not_bound_the_height() {
    // CSS 2.1 10.7: a percentage `max-height` against an indefinite
    // containing block height is treated as `none`.
    let style = Style {
        size: Size {
            width: taffy::Dimension::auto(),
            height: taffy::Dimension::length(100.0),
        },
        max_size: Size {
            width: taffy::LengthPercentageAuto::auto(),
            height: taffy::LengthPercentageAuto::percent(0.5),
        },
        ..Style::default()
    };
    let bounds = ifc_content_height_bounds(&style, None, None);
    assert_eq!(bounds, (0.0, f32::INFINITY));
    assert_eq!(
        ifc_definite_content_height(&style, None, taffy::Rect::zero(), bounds),
        Some(100.0)
    );
    assert_eq!(ifc_content_height_bounds(&style, Some(80.0), None).1, 40.0);
}
