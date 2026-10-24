//! An out-of-flow box whose `height` is `min-content`, `max-content` or
//! `fit-content` is not stretched between its `top` and `bottom`: it is
//! overconstrained, and the inset at the end of the containing block's inline
//! axis is ignored (CSS 2.1 §10.6.4, CSS Position 3). The sizes were checked
//! against Chromium.

use crate::document::Document;
use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

const CONTAINING_BLOCK: &str = "position:relative;width:200px;height:100px";

/// What the box under test holds.
#[derive(Clone, Copy)]
enum Content {
    /// One block 20px tall.
    Block,
    /// A block 50% tall and one 20px tall.
    PercentageBlock,
    /// A canvas of 50x50.
    Canvas,
}

/// The border box `(x, y, width, height)` of the child styled `child_css`,
/// relative to the border box of the root styled `root_css`.
fn placed(root_css: &str, child_css: &str, content: Content) -> (f32, f32, f32, f32) {
    let mut child = 0;
    let fixture = block_fixture(root_css, |doc, root| {
        let tag = if matches!(content, Content::Canvas) {
            "canvas"
        } else {
            "div"
        };
        child = doc.append_element(Some(root), tag, Some(&format!("display:block;{child_css}")));
        match content {
            Content::Block => {
                doc.append_element(Some(child), "div", Some("display:block;height:20px"));
            }
            Content::PercentageBlock => {
                doc.append_element(Some(child), "div", Some("display:block;height:50%"));
                doc.append_element(Some(child), "div", Some("display:block;height:20px"));
            }
            Content::Canvas => {
                doc.set_element_attribute(child, "width", "50")
                    .expect("set canvas width");
                doc.set_element_attribute(child, "height", "50")
                    .expect("set canvas height");
            }
        }
    });
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (root_x, root_y, ..) = absolute_rect(&doc, root);
    let (x, y, width, height) = absolute_rect(&doc, child);
    (x - root_x, y - root_y, width, height)
}

fn block_box(root_css: &str, child_css: &str) -> (f32, f32, f32, f32) {
    placed(root_css, child_css, Content::Block)
}

#[test]
fn a_keyword_height_takes_the_height_of_the_content_between_both_insets() {
    for position in ["absolute", "fixed"] {
        let root_css = if position == "fixed" {
            "width:200px;height:100px"
        } else {
            CONTAINING_BLOCK
        };
        for keyword in ["min-content", "max-content", "fit-content"] {
            assert_eq!(
                block_box(
                    root_css,
                    &format!(
                        "position:{position};top:0;bottom:0;left:0;width:50px;height:{keyword}"
                    ),
                ),
                (0.0, 0.0, 50.0, 20.0),
                "position:{position};height:{keyword}"
            );
        }
    }
}

#[test]
fn a_keyword_height_keeps_the_top_inset_and_the_margins_of_the_box() {
    assert_eq!(
        block_box(
            CONTAINING_BLOCK,
            "position:absolute;top:10px;bottom:15px;left:0;width:50px;height:max-content;\
             margin-top:5px;margin-bottom:7px",
        ),
        (0.0, 15.0, 50.0, 20.0)
    );
    assert_eq!(
        block_box(
            CONTAINING_BLOCK,
            "position:absolute;top:10%;bottom:calc(10% - 5px);left:0;width:50px;\
             height:max-content",
        ),
        (0.0, 10.0, 50.0, 20.0)
    );
}

#[test]
fn a_keyword_height_adds_the_padding_border_and_limits_of_the_box() {
    let child = "position:absolute;top:0;bottom:0;left:0;width:50px;height:max-content";
    assert_eq!(
        block_box(
            CONTAINING_BLOCK,
            &format!("{child};box-sizing:border-box;padding:3px 0;border:2px solid"),
        ),
        (0.0, 0.0, 50.0, 30.0)
    );
    assert_eq!(
        block_box(
            CONTAINING_BLOCK,
            &format!("{child};box-sizing:content-box;padding:3px 0;border:2px solid"),
        ),
        (0.0, 0.0, 54.0, 30.0)
    );
    assert_eq!(
        block_box(CONTAINING_BLOCK, &format!("{child};min-height:30px")),
        (0.0, 0.0, 50.0, 30.0)
    );
    assert_eq!(
        block_box(CONTAINING_BLOCK, &format!("{child};max-height:10px")),
        (0.0, 0.0, 50.0, 10.0)
    );
}

#[test]
fn a_box_without_a_keyword_height_still_fills_the_insets() {
    for height in ["auto", "fit-content(5px)"] {
        assert_eq!(
            block_box(
                CONTAINING_BLOCK,
                &format!("position:absolute;top:0;bottom:0;left:0;width:50px;height:{height}"),
            ),
            (0.0, 0.0, 50.0, 100.0),
            "height:{height}"
        );
    }
}

#[test]
fn a_keyword_height_makes_a_percentage_child_height_automatic() {
    assert_eq!(
        placed(
            CONTAINING_BLOCK,
            "position:absolute;top:0;bottom:0;left:0;width:50px;height:max-content",
            Content::PercentageBlock,
        ),
        (0.0, 0.0, 50.0, 20.0)
    );
}

#[test]
fn a_keyword_height_in_a_box_with_a_keyword_height_ignores_its_own_bottom_inset() {
    let mut inner = 0;
    let fixture = block_fixture(CONTAINING_BLOCK, |doc, root| {
        let outer = doc.append_element(
            Some(root),
            "div",
            Some("position:absolute;top:10px;bottom:10px;left:0;width:50px;height:min-content"),
        );
        inner = doc.append_element(
            Some(outer),
            "div",
            Some("position:absolute;top:0;bottom:0;left:0;width:10px;height:fit-content"),
        );
        doc.append_element(Some(inner), "div", Some("display:block;height:30px"));
        doc.append_element(Some(outer), "div", Some("display:block;height:5px"));
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let rect = absolute_rect(&doc, inner);
    assert_eq!((rect.2, rect.3), (10.0, 30.0));
}

#[test]
fn an_auto_margin_on_the_side_of_the_kept_inset_leaves_the_box_stretched() {
    let child = "position:absolute;top:0;bottom:0;left:0;width:50px;height:max-content";
    // The margin takes the free space of the box, which needs both insets.
    for margin in ["margin-top:auto", "margin:auto 0"] {
        assert_eq!(
            block_box(CONTAINING_BLOCK, &format!("{child};{margin}")),
            (0.0, 0.0, 50.0, 100.0),
            "{margin}"
        );
    }
    // The margin on the side of the ignored inset does not.
    assert_eq!(
        block_box(CONTAINING_BLOCK, &format!("{child};margin-bottom:auto")),
        (0.0, 0.0, 50.0, 20.0)
    );
    // The inline axis of the containing block starts at the bottom, so the
    // kept inset and the margin that matters are the bottom ones.
    let rtl = format!("{CONTAINING_BLOCK};writing-mode:vertical-rl;direction:rtl");
    assert_eq!(
        block_box(&rtl, &format!("{child};margin-top:auto")),
        (0.0, 80.0, 50.0, 20.0)
    );
    assert_eq!(
        block_box(&rtl, &format!("{child};margin-bottom:auto")),
        (0.0, 0.0, 50.0, 100.0)
    );
}

#[test]
fn a_box_aligned_to_the_middle_or_end_of_the_vertical_axis_keeps_both_insets() {
    let child = "position:absolute;top:10px;bottom:10px;left:0;width:50px;height:max-content";
    // The area between the insets is 80px tall and the box is 20px tall.
    for (alignment, y) in [
        ("auto", 10.0),
        ("normal", 10.0),
        ("stretch", 10.0),
        ("start", 10.0),
        ("flex-start", 10.0),
        ("baseline", 10.0),
        ("center", 40.0),
        ("end", 70.0),
        ("flex-end", 70.0),
        // An out-of-flow box takes no part in baseline alignment, and the
        // fallback alignment of the last baseline is the end.
        ("last baseline", 70.0),
    ] {
        assert_eq!(
            block_box(CONTAINING_BLOCK, &format!("{child};align-self:{alignment}")),
            (0.0, y, 50.0, 20.0),
            "align-self:{alignment}"
        );
    }
}

#[test]
fn only_the_alignment_of_the_vertical_axis_of_the_containing_block_applies() {
    let child = "position:absolute;top:10px;bottom:10px;left:0;width:50px;height:max-content";
    // `justify-self` is the horizontal axis of a horizontal containing block, and
    // the container's `align-items` does not apply to an out-of-flow box.
    assert_eq!(
        block_box(CONTAINING_BLOCK, &format!("{child};justify-self:center")),
        (0.0, 10.0, 50.0, 20.0)
    );
    assert_eq!(
        block_box(
            &format!("{CONTAINING_BLOCK};display:flex;align-items:center"),
            child
        ),
        (0.0, 10.0, 50.0, 20.0)
    );
    // The vertical axis of a vertical containing block is its inline axis, which
    // `align-self` does not align. The box sits at the start of that axis, the
    // bottom under `direction: rtl`.
    let vertical = format!("{CONTAINING_BLOCK};writing-mode:vertical-rl");
    for (root_css, y) in [
        (vertical.clone(), 10.0),
        (format!("{vertical};direction:rtl"), 70.0),
    ] {
        for alignment in ["align-self:center", "align-self:end", "justify-self:start"] {
            assert_eq!(
                block_box(&root_css, &format!("{child};{alignment}")),
                (0.0, y, 50.0, 20.0),
                "{root_css};{alignment}"
            );
        }
    }
}

#[test]
fn a_box_in_a_vertical_containing_block_keeps_both_insets_for_its_justify_self() {
    let child = "position:absolute;top:10px;bottom:10px;left:0;width:50px";
    // The inset that is kept is the one the box aligns to, so the box is laid
    // out like one whose height is `auto`.
    for direction in ["ltr", "rtl"] {
        let root_css = format!("{CONTAINING_BLOCK};writing-mode:vertical-rl;direction:{direction}");
        for alignment in ["center", "end", "flex-end", "last baseline"] {
            assert_eq!(
                block_box(
                    &root_css,
                    &format!("{child};height:max-content;justify-self:{alignment}")
                ),
                block_box(
                    &root_css,
                    &format!("{child};height:auto;justify-self:{alignment}")
                ),
                "{direction}; justify-self:{alignment}"
            );
        }
    }
}

#[test]
fn the_root_element_is_its_own_containing_block_for_a_keyword_height() {
    let mut doc = Document::new();
    let html = doc.append_element(
        Some(0),
        "html",
        Some("display:block;position:absolute;top:0;bottom:0;left:0;width:50px;height:max-content"),
    );
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    doc.append_element(Some(body), "div", Some("display:block;height:20px"));
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, html).3, 20.0);
}

#[test]
fn a_replaced_box_keeps_its_natural_height() {
    assert_eq!(
        placed(
            CONTAINING_BLOCK,
            "position:absolute;top:0;bottom:0;left:0;height:max-content",
            Content::Canvas,
        ),
        (0.0, 0.0, 50.0, 50.0)
    );
}

#[test]
fn the_inset_at_the_end_of_the_inline_axis_of_a_vertical_containing_block_is_ignored() {
    let child = "position:absolute;top:0;bottom:0;left:0;width:50px;height:max-content";
    // The inline axis runs from top to bottom, from bottom to top under
    // `direction: rtl`, and the other way round in `sideways-lr`. The direction
    // of a horizontal containing block does not matter.
    for (root_css, y) in [
        ("writing-mode:vertical-rl;direction:ltr", 0.0),
        ("writing-mode:vertical-rl;direction:rtl", 80.0),
        ("writing-mode:vertical-lr;direction:ltr", 0.0),
        ("writing-mode:vertical-lr;direction:rtl", 80.0),
        ("writing-mode:sideways-rl;direction:ltr", 0.0),
        ("writing-mode:sideways-rl;direction:rtl", 80.0),
        ("writing-mode:sideways-lr;direction:ltr", 80.0),
        ("writing-mode:sideways-lr;direction:rtl", 0.0),
        (
            "writing-mode:vertical-rl;direction:rtl;text-orientation:upright",
            80.0,
        ),
        ("writing-mode:horizontal-tb;direction:rtl", 0.0),
    ] {
        assert_eq!(
            block_box(&format!("{CONTAINING_BLOCK};{root_css}"), child),
            (0.0, y, 50.0, 20.0),
            "{root_css}"
        );
    }
}

#[test]
fn a_keyword_height_on_a_vertical_box_is_the_height_of_its_content() {
    let mut child = 0;
    let fixture = block_fixture(CONTAINING_BLOCK, |doc, root| {
        child = doc.append_element(
            Some(root),
            "div",
            Some(
                "position:absolute;top:0;bottom:0;left:0;width:30px;\
                 writing-mode:vertical-rl;height:max-content",
            ),
        );
        doc.append_element(
            Some(child),
            "div",
            Some("display:inline-block;width:30px;height:70px"),
        );
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let rect = absolute_rect(&doc, child);
    assert_eq!((rect.2, rect.3), (30.0, 70.0));
}
