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
