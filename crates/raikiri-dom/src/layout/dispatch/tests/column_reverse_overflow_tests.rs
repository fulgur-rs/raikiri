use crate::Document;
use crate::layout::context::LayoutSession;
use crate::layout::protocol::{AvailableSpace, LayoutInput, NodeId, RunMode};
use crate::layout::test_support::cascaded;
use raikiri_layout_geometry::Size;

// Border boxes recorded from Chromium 152.0.7977.82 in an 800x600 viewport,
// relative to the container. No page cuts are applied to these layout values.
#[test]
fn reversed_column_items_keep_their_size_when_they_overflow_above_the_container() {
    for (direction, styles, expected) in [
        (
            "column-reverse",
            vec!["height:300px;flex:none"],
            vec![(0.0, -200.0, 300.0, 300.0)],
        ),
        (
            "column-reverse",
            vec!["height:80px;flex:none", "height:80px;flex:none"],
            vec![(0.0, 20.0, 300.0, 80.0), (0.0, -60.0, 300.0, 80.0)],
        ),
        (
            "column-reverse",
            vec!["aspect-ratio:1/1;height:300px;flex:1 1 0"],
            vec![(0.0, -200.0, 300.0, 300.0)],
        ),
        (
            "column-reverse",
            vec!["height:60px;flex:none"],
            vec![(0.0, 40.0, 300.0, 60.0)],
        ),
        (
            "column",
            vec!["height:300px;flex:none"],
            vec![(0.0, 0.0, 300.0, 300.0)],
        ),
        (
            "row-reverse",
            vec!["width:500px;flex:none"],
            vec![(-200.0, 0.0, 500.0, 100.0)],
        ),
    ] {
        let mut doc = Document::new();
        let parent = doc.append_element(
            Some(0),
            "div",
            Some(&format!(
                "display:flex;flex-direction:{direction};width:300px;height:100px"
            )),
        );
        let children: Vec<_> = styles
            .iter()
            .map(|style| doc.append_element(Some(parent), "div", Some(*style)))
            .collect();
        doc.mark_in_document_flags();
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        tree.compute_child_layout(
            NodeId::from(parent),
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                known_dimensions: Size {
                    width: Some(300.0),
                    height: Some(100.0),
                },
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size: Size {
                    width: Some(800.0),
                    height: Some(600.0),
                },
                available_space: Size {
                    width: AvailableSpace::Definite(300.0),
                    height: AvailableSpace::Definite(100.0),
                },
                ..LayoutInput::HIDDEN
            },
        );
        for (child, expected) in children.into_iter().zip(expected) {
            let actual = tree.nodes[child].layout;
            assert_eq!(
                (
                    actual.location.x,
                    actual.location.y,
                    actual.size.width,
                    actual.size.height
                ),
                expected,
                "{direction}: {styles:?}"
            );
        }
    }
}
