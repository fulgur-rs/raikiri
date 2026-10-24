//! The scrollable overflow that the absolutely positioned children of flex and
//! grid containers on the axes of a vertical writing mode add to their
//! containing block.
//!
//! The rectangle is in physical coordinates, measured from the top left corner
//! of the padding box. A reversed inline direction reverses the vertical axis
//! of a vertical container, never the horizontal one.

use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{page_box_800x600, with_ahem};

/// The scrollable overflow of the positioned 100px square container `#c`,
/// displayed as `display` and styled `container_css`, as (left, top, right,
/// bottom), once it has laid out a box styled `child_css`.
fn overflow(display: &str, container_css: &str, child_css: &str) -> (f32, f32, f32, f32) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let container = doc.append_element(
        Some(body),
        "div",
        Some(
            format!("display:{display};position:relative;width:100px;height:100px;{container_css}")
                .as_str(),
        ),
    );
    doc.append_element(
        Some(container),
        "div",
        Some(format!("display:block;position:absolute;{child_css}").as_str()),
    );
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let rect = doc.nodes[container].layout.scrollable_overflow_rect;
    (rect.left, rect.top, rect.right, rect.bottom)
}

/// A 50px square box that sticks out of the container by 30px on the right
/// and by 40px at the bottom.
const STICKING_OUT: &str = "left:80px;right:-30px;top:90px;bottom:-40px;width:50px;height:50px";

#[test]
fn the_overflow_of_a_box_in_a_vertical_flex_container_is_measured_from_the_top_left() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for direction in ["ltr", "rtl"] {
            assert_eq!(
                overflow(
                    "flex",
                    &format!("writing-mode:{mode};direction:{direction}"),
                    STICKING_OUT
                ),
                (0.0, 0.0, 130.0, 140.0),
                "{mode} {direction}"
            );
        }
    }
}

#[test]
fn the_overflow_of_a_box_in_a_vertical_grid_container_is_measured_from_the_top_left() {
    for mode in ["vertical-rl", "vertical-lr"] {
        for direction in ["ltr", "rtl"] {
            assert_eq!(
                overflow(
                    "grid",
                    &format!("writing-mode:{mode};direction:{direction}"),
                    STICKING_OUT
                ),
                (0.0, 0.0, 130.0, 140.0),
                "{mode} {direction}"
            );
        }
    }
}

#[test]
fn the_overflow_of_a_box_before_the_start_edges_reaches_a_negative_coordinate() {
    // The box sticks out of the container by 20px at the left and by 10px at
    // the top, whatever the direction. The container has no in-flow content,
    // so the overflow is that of the box alone.
    let child = "left:-20px;right:70px;top:-10px;bottom:60px;width:50px;height:50px";
    for direction in ["ltr", "rtl"] {
        assert_eq!(
            overflow(
                "flex",
                &format!("writing-mode:vertical-rl;direction:{direction}"),
                child
            ),
            (-20.0, -10.0, 30.0, 40.0),
            "{direction}"
        );
    }
}
