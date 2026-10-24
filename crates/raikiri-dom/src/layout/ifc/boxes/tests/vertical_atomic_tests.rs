//! An atomic inline in a vertical paragraph sits on the line it belongs to:
//! its logical position along the line and across the lines is mapped to the
//! physical box through the paragraph's writing mode and direction.
//!
//! The expected boxes of the baseline-aligned cases are Chromium's.

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// The border box of a 10px square inline-block that follows the text "aa" in
/// a 10px Ahem paragraph styled by `root_css`.
fn atomic_rect(root_css: &str) -> (f32, f32, f32, f32) {
    atomic_rect_styled(root_css, "width:10px;height:10px")
}

/// The border box of an inline-block styled by `atomic_css` that follows the
/// text "aa" in a 10px Ahem paragraph styled by `root_css`.
fn atomic_rect_styled(root_css: &str, atomic_css: &str) -> (f32, f32, f32, f32) {
    let mut atomic = 0;
    let fixture = block_fixture(&format!("line-height:10px;{root_css}"), |doc, root| {
        doc.append_text(root, "aa");
        atomic = doc.append_element(
            Some(root),
            "div",
            Some(format!("display:inline-block;{atomic_css}").as_str()),
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

#[test]
fn an_atomic_takes_its_width_across_the_lines_and_its_height_along_them() {
    // The 20px wide, 10px tall box makes the line 20px wide, the whole right
    // column, and follows the text down it for 10px.
    assert_eq!(
        atomic_rect_styled(
            "writing-mode:vertical-rl;width:100px;height:100px",
            "width:20px;height:10px"
        ),
        (80.0, 20.0, 20.0, 10.0)
    );
    assert_eq!(
        atomic_rect_styled(
            "writing-mode:vertical-lr;width:100px;height:100px",
            "width:20px;height:10px"
        ),
        (0.0, 20.0, 20.0, 10.0)
    );
}

#[test]
fn the_margins_of_an_atomic_in_a_vertical_line_are_mapped_to_its_logical_sides() {
    // The top margin lies before the box along the line and the right one
    // at the start of the lines' block axis: the 20px box has 2px at the
    // right edge and 4px at the left of the 26px line.
    assert_eq!(
        atomic_rect_styled(
            "writing-mode:vertical-rl;width:100px;height:100px",
            "width:20px;height:10px;margin:1px 2px 3px 4px"
        ),
        (78.0, 21.0, 20.0, 10.0)
    );
    // A vertical-lr line starts at the left edge, with the left margin.
    assert_eq!(
        atomic_rect_styled(
            "writing-mode:vertical-lr;width:100px;height:100px",
            "width:20px;height:10px;margin:1px 2px 3px 4px"
        ),
        (4.0, 21.0, 20.0, 10.0)
    );
}

#[test]
fn the_margins_of_an_atomic_in_a_right_to_left_vertical_line_start_at_the_bottom() {
    // The line runs up from the bottom edge, so the bottom margin is the
    // inline-start one: the text takes the last 20px, then 3px of margin,
    // the 10px box and 1px of margin.
    assert_eq!(
        atomic_rect_styled(
            "writing-mode:vertical-rl;direction:rtl;width:100px;height:100px",
            "width:20px;height:10px;margin:1px 2px 3px 4px"
        ),
        (78.0, 67.0, 20.0, 10.0)
    );
}

#[test]
fn an_atomic_resolves_a_percentage_width_against_the_physical_width_of_a_vertical_root() {
    // The 100px wide, 50px tall root is the containing block: 50% of its
    // width is 50px and 20% of its height 10px. The line is as wide as the
    // box and sits at the right edge, with the text above the box.
    for mode in ["vertical-rl", "vertical-lr"] {
        let (x, y, width, height) = atomic_rect_styled(
            &format!("writing-mode:{mode};width:100px;height:50px"),
            "width:50%;height:20%",
        );
        assert_eq!(
            (y, width, height),
            (20.0, 50.0, 10.0),
            "{mode}: the box is 50x10 and follows the text"
        );
        assert_eq!(x, if mode == "vertical-rl" { 50.0 } else { 0.0 }, "{mode}");
    }
}

#[test]
fn a_shrink_to_fit_vertical_root_counts_the_height_of_an_atomic_along_its_lines() {
    // The text takes 20px along the line and the box 10px: the float is 30px
    // tall, not 20px plus the 20px width of the box.
    let mut atomic = 0;
    let fixture = block_fixture(
        "line-height:10px;writing-mode:vertical-rl;float:left",
        |doc, root| {
            doc.append_text(root, "aa");
            atomic = doc.append_element(
                Some(root),
                "div",
                Some("display:inline-block;width:20px;height:10px"),
            );
        },
    );
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let size = doc.nodes[fixture.root].layout.size;
    assert_eq!((size.width, size.height), (20.0, 30.0));
    assert_eq!(absolute_rect(&doc, atomic).3, 10.0);
}

#[test]
fn the_percentage_edges_of_an_atomic_in_a_vertical_root_resolve_against_its_inline_size() {
    // Padding takes its percentage of the 50px inline size, the root's
    // height, while the width takes its own of the 100px physical width.
    let size = |atomic_css: &str| {
        let (_, _, width, height) = atomic_rect_styled(
            "writing-mode:vertical-rl;width:100px;height:50px",
            &format!("width:20px;height:10px;{atomic_css}"),
        );
        (width, height)
    };
    assert_eq!(size("padding:10%"), (30.0, 20.0));
    assert_eq!(size("padding:10%;width:50%"), (60.0, 20.0));
    // With `border-box` the padding is part of the sizes.
    assert_eq!(
        size("padding:10%;width:50%;box-sizing:border-box"),
        (50.0, 10.0)
    );
}

/// The border boxes of inline-blocks holding the text "a", each styled by one
/// entry of `atomics`, in a sideways 10px Ahem paragraph of a 100px square
/// styled by `root_css`. Sideways text aligns atomics on the alphabetic
/// baseline.
fn text_atomic_rects(root_css: &str, atomics: &[&str]) -> Vec<(f32, f32, f32, f32)> {
    let mut ids = Vec::new();
    let fixture = block_fixture(
        &format!("line-height:10px;text-orientation:sideways;width:100px;height:100px;{root_css}"),
        |doc, root| {
            for css in atomics {
                let atomic = doc.append_element(
                    Some(root),
                    "div",
                    Some(format!("display:inline-block;{css}").as_str()),
                );
                doc.append_text(atomic, "a");
                ids.push(atomic);
            }
        },
    );
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    ids.into_iter().map(|id| absolute_rect(&doc, id)).collect()
}

/// A 20px box whose text has the baseline 16px from its line-over edge.
const BIG: &str = "font-size:20px;line-height:20px";

#[test]
fn atomics_in_vertical_rl_share_the_baseline_of_their_text() {
    // The 10px text's baseline is 8px from its right edge and the 20px
    // text's 16px, so the small box sits 8px left of the big one's right
    // edge.
    assert_eq!(
        text_atomic_rects("writing-mode:vertical-rl", &[BIG, ""]),
        [(80.0, 0.0, 20.0, 20.0), (82.0, 20.0, 10.0, 10.0)]
    );
}

#[test]
fn the_margin_at_the_line_over_edge_of_an_atomic_moves_its_baseline_in_vertical_rl() {
    // The right margin lies on the line-over side of the baseline: with 20px
    // of it the small box reaches 28px above the baseline, past the big
    // one's 16px, and the baseline moves to 28px from the right edge.
    assert_eq!(
        text_atomic_rects("writing-mode:vertical-rl", &["margin:0 20px 0 0", BIG]),
        [(70.0, 0.0, 10.0, 10.0), (68.0, 10.0, 20.0, 20.0)]
    );
    // The left margin lies on the line-under side, which does not reach past
    // the baseline's 16px from the right edge.
    assert_eq!(
        text_atomic_rects("writing-mode:vertical-rl", &["margin:0 0 0 20px", BIG]),
        [(82.0, 0.0, 10.0, 10.0), (80.0, 10.0, 20.0, 20.0)]
    );
    // The top and bottom margins lie along the line.
    assert_eq!(
        text_atomic_rects(
            "writing-mode:vertical-rl",
            &[
                "margin:3px 6px 5px 7px",
                "font-size:20px;line-height:20px;margin:1px 2px 3px 4px"
            ]
        ),
        [(80.0, 3.0, 10.0, 10.0), (78.0, 19.0, 20.0, 20.0)]
    );
}

#[test]
fn an_atomic_whose_lines_run_across_the_paragraphs_has_no_baseline_to_share() {
    // A horizontal box and a box with an opposite block direction sit on
    // their line-under edge, the left edge of the margin box, which is the
    // baseline of the big box here.
    assert_eq!(
        text_atomic_rects(
            "writing-mode:vertical-rl",
            &["writing-mode:horizontal-tb", BIG]
        ),
        [(84.0, 0.0, 10.0, 10.0), (80.0, 10.0, 20.0, 20.0)]
    );
    assert_eq!(
        text_atomic_rects(
            "writing-mode:vertical-rl",
            &["writing-mode:vertical-lr;margin:0 3px 0 5px", BIG]
        ),
        [(87.0, 0.0, 10.0, 10.0), (78.0, 10.0, 20.0, 20.0)]
    );
}

#[test]
fn a_scroll_container_atomic_has_no_baseline_in_a_vertical_line() {
    assert_eq!(
        text_atomic_rects(
            "writing-mode:vertical-rl",
            &["overflow:hidden;width:15px;height:15px", BIG]
        ),
        [(84.0, 0.0, 15.0, 15.0), (80.0, 15.0, 20.0, 20.0)]
    );
}

#[test]
fn atomics_in_vertical_lr_share_the_baseline_of_their_text() {
    // Text in vertical-lr is rotated like text in vertical-rl, so the line-over
    // edge is the right one here too: the small box sits 8px left of the
    // right edge it would share with the big one.
    assert_eq!(
        text_atomic_rects("writing-mode:vertical-lr", &[BIG, ""]),
        [(0.0, 0.0, 20.0, 20.0), (2.0, 20.0, 10.0, 10.0)]
    );
}

#[test]
fn the_margin_at_the_line_over_edge_of_an_atomic_moves_its_baseline_in_vertical_lr() {
    assert_eq!(
        text_atomic_rects("writing-mode:vertical-lr", &["margin:0 20px 0 0", BIG]),
        [(2.0, 0.0, 10.0, 10.0), (0.0, 10.0, 20.0, 20.0)]
    );
    assert_eq!(
        text_atomic_rects("writing-mode:vertical-lr", &["margin:0 0 0 20px", BIG]),
        [(20.0, 0.0, 10.0, 10.0), (18.0, 10.0, 20.0, 20.0)]
    );
    assert_eq!(
        text_atomic_rects(
            "writing-mode:vertical-lr",
            &[
                "margin:3px 6px 5px 7px",
                "font-size:20px;line-height:20px;margin:1px 2px 3px 4px"
            ]
        ),
        [(7.0, 3.0, 10.0, 10.0), (5.0, 19.0, 20.0, 20.0)]
    );
}

#[test]
fn an_atomic_with_no_baseline_to_share_sits_on_the_line_under_edge_in_vertical_lr() {
    assert_eq!(
        text_atomic_rects(
            "writing-mode:vertical-lr",
            &["writing-mode:horizontal-tb", BIG]
        ),
        [(4.0, 0.0, 10.0, 10.0), (0.0, 10.0, 20.0, 20.0)]
    );
    assert_eq!(
        text_atomic_rects(
            "writing-mode:vertical-lr",
            &["writing-mode:vertical-rl;margin:0 3px 0 5px", BIG]
        ),
        [(9.0, 0.0, 10.0, 10.0), (0.0, 10.0, 20.0, 20.0)]
    );
    assert_eq!(
        text_atomic_rects(
            "writing-mode:vertical-lr",
            &["overflow:hidden;width:15px;height:15px", BIG]
        ),
        [(4.0, 0.0, 15.0, 15.0), (0.0, 15.0, 20.0, 20.0)]
    );
}

#[test]
fn atomics_in_sideways_lr_share_the_baseline_measured_from_the_left_edge() {
    // The line-over edge is the left one and the text runs up the line, so
    // the first box is at the bottom and the small one's baseline, 8px from
    // its left edge, meets the big one's, 16px from its.
    assert_eq!(
        text_atomic_rects("writing-mode:sideways-lr", &[BIG, ""]),
        [(0.0, 80.0, 20.0, 20.0), (8.0, 70.0, 10.0, 10.0)]
    );
    // The left margin lies on the line-over side of the baseline, the right
    // margin on the line-under side.
    assert_eq!(
        text_atomic_rects("writing-mode:sideways-lr", &["margin:0 0 0 20px", BIG]),
        [(20.0, 90.0, 10.0, 10.0), (12.0, 70.0, 20.0, 20.0)]
    );
    assert_eq!(
        text_atomic_rects("writing-mode:sideways-lr", &["margin:0 20px 0 0", BIG]),
        [(8.0, 90.0, 10.0, 10.0), (0.0, 70.0, 20.0, 20.0)]
    );
}

#[test]
fn an_atomic_that_is_not_laid_out_like_a_sideways_lr_line_sits_on_its_right_edge() {
    // Its margin box's right edge is the line-under edge, which is the big
    // box's baseline, 16px from its left edge.
    for atomic in [
        "writing-mode:horizontal-tb",
        "writing-mode:vertical-lr",
        "writing-mode:sideways-rl",
    ] {
        assert_eq!(
            text_atomic_rects("writing-mode:sideways-lr", &[atomic, BIG]),
            [(6.0, 90.0, 10.0, 10.0), (0.0, 70.0, 20.0, 20.0)],
            "{atomic}"
        );
    }
    assert_eq!(
        text_atomic_rects(
            "writing-mode:sideways-lr",
            &["writing-mode:vertical-rl;margin:0 3px 0 5px", BIG]
        ),
        [(5.0, 90.0, 10.0, 10.0), (2.0, 70.0, 20.0, 20.0)]
    );
}
