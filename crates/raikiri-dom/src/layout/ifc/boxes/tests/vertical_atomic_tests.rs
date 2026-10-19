//! An atomic inline in a vertical paragraph sits on the line it belongs to:
//! its logical position along the line and across the lines is mapped to the
//! physical box through the paragraph's writing mode and direction.

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// The border box of a 10px square inline-block that follows the text "aa" in
/// a 10px Ahem paragraph styled by `root_css`.
fn atomic_rect(root_css: &str) -> (f32, f32, f32, f32) {
    let mut atomic = 0;
    let fixture = block_fixture(&format!("line-height:10px;{root_css}"), |doc, root| {
        doc.append_text(root, "aa");
        atomic = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some("display:inline-block;width:10px;height:10px"),
        );
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    absolute_rect(&doc, atomic)
}

#[test]
fn an_atomic_in_vertical_rl_follows_the_text_down_the_line_at_the_right_edge() {
    // The line is the 10px column at the right edge of the 100px root; the
    // text takes its first 20px and the box the next 10px.
    assert_eq!(
        atomic_rect("writing-mode:vertical-rl;width:100px;height:100px"),
        (90.0, 20.0, 10.0, 10.0)
    );
}

#[test]
fn an_atomic_in_vertical_lr_follows_the_text_down_the_line_at_the_left_edge() {
    assert_eq!(
        atomic_rect("writing-mode:vertical-lr;width:100px;height:100px"),
        (0.0, 20.0, 10.0, 10.0)
    );
}

#[test]
fn an_atomic_on_the_second_line_of_vertical_rl_is_left_of_the_first_line() {
    // The 20px height leaves room for the text alone, so the box starts the
    // second line, which is the column left of the first.
    assert_eq!(
        atomic_rect("writing-mode:vertical-rl;width:100px;height:20px"),
        (80.0, 0.0, 10.0, 10.0)
    );
}

#[test]
fn an_atomic_on_the_second_line_of_vertical_lr_is_right_of_the_first_line() {
    assert_eq!(
        atomic_rect("writing-mode:vertical-lr;width:100px;height:20px"),
        (10.0, 0.0, 10.0, 10.0)
    );
}

#[test]
fn an_atomic_in_a_right_to_left_vertical_line_follows_the_text_up_the_line() {
    // The inline axis runs from the bottom edge: the text takes the last 20px
    // and the box the 10px above it.
    assert_eq!(
        atomic_rect("writing-mode:vertical-rl;direction:rtl;width:100px;height:100px"),
        (90.0, 70.0, 10.0, 10.0)
    );
}

#[test]
fn an_atomic_in_an_auto_width_vertical_root_uses_the_width_the_lines_give_it() {
    // The root is as wide as its two 10px lines. The box starts the second
    // line, the column at the root's left edge.
    assert_eq!(
        atomic_rect("writing-mode:vertical-rl;float:left;height:20px"),
        (0.0, 0.0, 10.0, 10.0)
    );
}
