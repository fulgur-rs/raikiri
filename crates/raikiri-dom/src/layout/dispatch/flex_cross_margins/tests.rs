use super::*;
use crate::Document;
use crate::layout::protocol::{AvailableSpace, RunMode};
use crate::layout::taffy_adapter::algorithms::compute_flexbox_layout;
use crate::layout::test_support::cascaded;
use raikiri_layout_geometry::Size;

#[test]
fn scroll_reach_and_used_margins_follow_the_corrected_border_box() {
    for (direction, text_direction, location, end) in [
        ("column", "ltr", Point::ZERO, Point { x: 200.0, y: 20.0 }),
        (
            "column",
            "rtl",
            Point { x: -100.0, y: 0.0 },
            Point { x: 200.0, y: 20.0 },
        ),
        ("row", "ltr", Point::ZERO, Point { x: 20.0, y: 200.0 }),
    ] {
        let row = direction == "row";
        let mut doc = Document::new();
        let root = doc.append_element(Some(0), "div", Some(&format!("display:flex;flex-direction:{direction};direction:{text_direction};width:100px;height:100px;overflow:auto")));
        let child = doc.append_element(
            Some(root),
            "div",
            Some(if row {
                "width:20px;height:200px;flex:none;margin:auto 0"
            } else {
                "width:200px;height:20px;flex:none;margin:0 auto"
            }),
        );
        doc.mark_in_document_flags();
        let cascade = cascaded(&doc);
        let mut tree = LayoutSession::bridged(&mut doc, &cascade);
        let inputs = LayoutInput {
            run_mode: RunMode::PerformLayout,
            known_dimensions: Size {
                width: Some(100.0),
                height: Some(100.0),
            },
            parent_size: Size {
                width: Some(800.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(100.0),
                height: AvailableSpace::Definite(100.0),
            },
            ..LayoutInput::HIDDEN
        };
        let output = super::super::table_items::layout(
            &mut tree,
            root.into(),
            inputs,
            compute_flexbox_layout,
        );
        let layout = tree.nodes[child].layout;
        assert_eq!(layout.location, location);
        assert_eq!(
            output.scrollable_overflow_rect,
            Edges {
                left: 0.0,
                top: 0.0,
                right: end.x,
                bottom: end.y
            }
        );
        if row {
            assert_eq!((layout.margin.top, layout.margin.bottom), (0.0, -100.0));
        } else if text_direction == "rtl" {
            assert_eq!((layout.margin.left, layout.margin.right), (-100.0, 0.0));
        } else {
            assert_eq!((layout.margin.left, layout.margin.right), (0.0, -100.0));
        }
    }
}
