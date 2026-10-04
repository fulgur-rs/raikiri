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
