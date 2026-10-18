//! The fixed dimension of margin boxes (CSS Page 3 §5.3.3): the height of top
//! and bottom boxes, the width of left and right boxes, and both dimensions of
//! corner boxes. The page is 400x300; its page margins are 40px unless the
//! test sets them.

use super::layout_tests::{boxes_for, rects};
use super::*;

/// The `[x, y, width, height]` of the only box of `slot` on a page whose
/// margins are `margin`.
fn lone_box(margin: &str, slot: &str, declarations: &str) -> [f32; 4] {
    let boxes = boxes_for(&format!(
        "@page {{ margin: {margin}; @{slot} {{ content: ''; {declarations} }} }}"
    ));
    let [(_, rect)] = rects(&boxes)[..] else {
        panic!("one box: {boxes:?}");
    };
    rect
}

/// [`lone_box`] on a page whose margins are 40px.
fn in_40px_margins(slot: &str, declarations: &str) -> [f32; 4] {
    lone_box("40px", slot, declarations)
}

#[test]
fn a_top_box_with_both_margins_auto_is_centered_in_the_page_margin() {
    assert_eq!(
        in_40px_margins(
            "top-center",
            "height: 10px; margin-top: auto; margin-bottom: auto"
        ),
        [40.0, 15.0, 320.0, 10.0]
    );
    // The border and the padding are part of what is centered.
    assert_eq!(
        in_40px_margins(
            "top-center",
            "height: 10px; border: 2px solid; padding: 3px; margin-top: auto; margin-bottom: auto"
        ),
        [40.0, 10.0, 320.0, 20.0]
    );
}

#[test]
fn a_top_box_with_one_auto_margin_takes_what_the_other_leaves() {
    assert_eq!(
        in_40px_margins(
            "top-center",
            "height: 10px; margin-top: auto; margin-bottom: 5px"
        ),
        [40.0, 25.0, 320.0, 10.0]
    );
    assert_eq!(
        in_40px_margins(
            "top-center",
            "height: 10px; margin-top: 8px; margin-bottom: auto"
        ),
        [40.0, 8.0, 320.0, 10.0]
    );
    // The borders and paddings count as well.
    assert_eq!(
        in_40px_margins(
            "top-center",
            "height: 10px; border: 2px solid; padding: 3px; margin-top: auto; margin-bottom: 1px"
        ),
        [40.0, 19.0, 320.0, 20.0]
    );
}

#[test]
fn an_over_constrained_top_box_drops_its_top_margin() {
    // The box ends up against the page area, with its bottom margin kept.
    assert_eq!(
        in_40px_margins(
            "top-center",
            "height: 10px; margin-top: 4px; margin-bottom: 6px"
        ),
        [40.0, 24.0, 320.0, 10.0]
    );
    // A box with no margins at all sits at the bottom of the page margin too.
    assert_eq!(
        in_40px_margins("top-center", "height: 10px"),
        [40.0, 30.0, 320.0, 10.0]
    );
}

#[test]
fn an_over_constrained_bottom_box_drops_its_bottom_margin() {
    // The bottom page margin runs from y = 260 to y = 300. The box ends up
    // against the page area, with its top margin kept.
    assert_eq!(
        in_40px_margins(
            "bottom-center",
            "height: 10px; margin-top: 4px; margin-bottom: 6px"
        ),
        [40.0, 264.0, 320.0, 10.0]
    );
    assert_eq!(
        in_40px_margins("bottom-center", "height: 10px"),
        [40.0, 260.0, 320.0, 10.0]
    );
}

#[test]
fn a_box_taller_than_its_page_margin_overflows_away_from_the_page_area() {
    // The dropped margin goes negative, which keeps the box against the page
    // area.
    assert_eq!(
        in_40px_margins("top-center", "height: 60px"),
        [40.0, -20.0, 320.0, 60.0]
    );
    assert_eq!(
        in_40px_margins("bottom-center", "height: 60px"),
        [40.0, 260.0, 320.0, 60.0]
    );
}

#[test]
fn auto_margins_become_zero_where_the_box_already_overflows_the_page_margin() {
    // A 25px square with 3px borders and a 90px margin on the side away from
    // the page area, in a 100px page margin: the auto margin on the other
    // side would be negative, so it is zero, and the box is over-constrained.
    // Its margin on the page-edge side gives way, which puts the box against
    // the page area.
    let square = "width: 25px; height: 25px; border: 3px solid; margin: auto;";
    let [_, y, _, height] = lone_box("100px", "top-center", &format!("{square} margin-top: 90px"));
    assert_eq!([y, height], [69.0, 31.0]);
    let [_, y, _, height] = lone_box(
        "100px",
        "bottom-center",
        &format!("{square} margin-bottom: 90px"),
    );
    assert_eq!([y, height], [200.0, 31.0]);
    let [x, _, width, _] = lone_box(
        "100px",
        "left-middle",
        &format!("{square} margin-left: 90px"),
    );
    assert_eq!([x, width], [69.0, 31.0]);
    let [x, _, width, _] = lone_box(
        "100px",
        "right-middle",
        &format!("{square} margin-right: 90px"),
    );
    assert_eq!([x, width], [300.0, 31.0]);
}

#[test]
fn an_auto_height_fills_the_page_margin_inside_its_margins() {
    assert_eq!(
        in_40px_margins("top-center", "margin-top: 5px; margin-bottom: 3px"),
        [40.0, 5.0, 320.0, 32.0]
    );
    // An auto margin claims nothing when the height is auto.
    assert_eq!(
        in_40px_margins("top-center", "margin-top: auto; margin-bottom: 3px"),
        [40.0, 0.0, 320.0, 37.0]
    );
    assert_eq!(
        in_40px_margins("top-center", "margin-top: auto; margin-bottom: auto"),
        [40.0, 0.0, 320.0, 40.0]
    );
    // The borders are inside the height.
    assert_eq!(
        in_40px_margins("top-center", "border: 2px solid; margin-top: 5px"),
        [40.0, 5.0, 320.0, 35.0]
    );
}

#[test]
fn an_auto_height_never_goes_negative() {
    let boxes = boxes_for(
        "@page { margin: 40px; @top-center { content: ''; margin-top: 30px; margin-bottom: 30px } }",
    );
    assert_eq!(rects(&boxes), []);
}

#[test]
fn an_auto_size_that_would_be_negative_over_constrains_the_box() {
    // The padding alone is taller than the 40px page margin, so the height is
    // zero and the margin towards the page edge gives way, whether the
    // margins are auto or not. The overflow points away from the page area.
    let padding = "padding-top: 30px; padding-bottom: 30px;";
    for margins in ["margin-top: auto; margin-bottom: auto", ""] {
        assert_eq!(
            in_40px_margins("top-center", &format!("{padding} {margins}")),
            [40.0, -20.0, 320.0, 60.0],
            "{margins}"
        );
        assert_eq!(
            in_40px_margins("bottom-center", &format!("{padding} {margins}")),
            [40.0, 260.0, 320.0, 60.0],
            "{margins}"
        );
    }
    // The margin on the side of the page area is kept.
    assert_eq!(
        in_40px_margins(
            "top-center",
            &format!("{padding} margin-top: 5px; margin-bottom: 3px")
        ),
        [40.0, -23.0, 320.0, 60.0]
    );
    assert_eq!(
        in_40px_margins(
            "bottom-center",
            &format!("{padding} margin-top: 5px; margin-bottom: 3px")
        ),
        [40.0, 265.0, 320.0, 60.0]
    );
    // The same goes for the width of side boxes.
    let padding = "padding-left: 30px; padding-right: 30px;";
    assert_eq!(
        in_40px_margins("left-middle", padding),
        [-20.0, 40.0, 60.0, 220.0]
    );
    assert_eq!(
        in_40px_margins("right-middle", padding),
        [360.0, 40.0, 60.0, 220.0]
    );
}

#[test]
fn a_side_box_solves_its_width_like_a_top_box_solves_its_height() {
    // With both margins auto it is centered in the 40px column.
    assert_eq!(
        in_40px_margins(
            "left-middle",
            "width: 10px; margin-left: auto; margin-right: auto"
        ),
        [15.0, 40.0, 10.0, 220.0]
    );
    assert_eq!(
        in_40px_margins(
            "right-middle",
            "width: 10px; margin-left: auto; margin-right: auto"
        ),
        [375.0, 40.0, 10.0, 220.0]
    );
    // With one it takes what the other leaves.
    assert_eq!(
        in_40px_margins(
            "left-middle",
            "width: 10px; margin-left: auto; margin-right: 5px"
        ),
        [25.0, 40.0, 10.0, 220.0]
    );
    assert_eq!(
        in_40px_margins(
            "right-middle",
            "width: 10px; margin-left: 5px; margin-right: auto"
        ),
        [365.0, 40.0, 10.0, 220.0]
    );
    // An auto width fills the column inside its margins.
    assert_eq!(
        in_40px_margins("left-middle", "margin-left: 5px; margin-right: 3px"),
        [5.0, 40.0, 32.0, 220.0]
    );
}

#[test]
fn an_over_constrained_side_box_drops_the_margin_towards_the_page_edge() {
    // A left box drops its left margin and a right box its right margin, so
    // each ends up against the page area.
    assert_eq!(
        in_40px_margins(
            "left-middle",
            "width: 10px; margin-left: 4px; margin-right: 6px"
        ),
        [24.0, 40.0, 10.0, 220.0]
    );
    assert_eq!(
        in_40px_margins(
            "right-middle",
            "width: 10px; margin-left: 4px; margin-right: 6px"
        ),
        [364.0, 40.0, 10.0, 220.0]
    );
    assert_eq!(
        in_40px_margins("left-middle", "width: 60px"),
        [-20.0, 40.0, 60.0, 220.0]
    );
    assert_eq!(
        in_40px_margins("right-middle", "width: 60px"),
        [360.0, 40.0, 60.0, 220.0]
    );
}

#[test]
fn a_corner_box_solves_both_of_its_dimensions() {
    let square = "width: 10px; height: 10px;";
    // Over-constrained in both dimensions: the margins towards the page edges
    // give way, which puts each box in the corner of its cell next to the
    // page area.
    for (slot, expected) in [
        ("top-left-corner", [28.0, 28.0]),
        ("top-right-corner", [362.0, 28.0]),
        ("bottom-left-corner", [28.0, 262.0]),
        ("bottom-right-corner", [362.0, 262.0]),
    ] {
        assert_eq!(
            in_40px_margins(slot, &format!("{square} margin: 2px"))[..2],
            expected,
            "{slot}"
        );
    }
    // Auto margins center the box in both dimensions.
    assert_eq!(
        in_40px_margins("top-left-corner", &format!("{square} margin: auto")),
        [15.0, 15.0, 10.0, 10.0]
    );
    assert_eq!(
        in_40px_margins("bottom-right-corner", &format!("{square} margin: auto")),
        [375.0, 275.0, 10.0, 10.0]
    );
    // An auto size fills the cell inside the margins.
    assert_eq!(
        in_40px_margins("top-left-corner", "margin: 3px"),
        [3.0, 3.0, 34.0, 34.0]
    );
    // One auto margin takes what the other leaves, in each dimension on its own.
    assert_eq!(
        in_40px_margins(
            "top-right-corner",
            &format!(
                "{square} margin-left: auto; margin-right: 5px; margin-top: 4px; margin-bottom: auto"
            )
        ),
        [385.0, 4.0, 10.0, 10.0]
    );
}

#[test]
fn a_corner_box_keeps_its_margins_when_it_overflows_its_cell() {
    // The margins are part of the size a corner box is limited to, not
    // swallowed by it.
    let square = "width: 25px; height: 25px; border: 3px solid; margin: auto;";
    assert_eq!(
        lone_box(
            "100px",
            "top-left-corner",
            &format!("{square} margin-top: 90px; margin-left: 90px")
        ),
        [69.0, 69.0, 31.0, 31.0]
    );
    assert_eq!(
        lone_box(
            "100px",
            "bottom-right-corner",
            &format!("{square} margin-bottom: 90px; margin-right: 90px")
        ),
        [300.0, 200.0, 31.0, 31.0]
    );
}

#[test]
fn a_corner_of_a_page_margin_with_no_size_has_no_box() {
    for margin in ["40px 40px 40px 0", "0 40px 40px 40px"] {
        let boxes = boxes_for(&format!(
            "@page {{ margin: {margin}; \
               @top-left-corner {{ content: ''; width: 10px; height: 10px }} }}"
        ));
        assert_eq!(rects(&boxes), [], "{margin}");
    }
}

#[test]
fn the_content_size_does_not_depend_on_which_margin_gives_way() {
    // The text of a box is measured before it is laid out, without knowing
    // which edge it is on.
    for size in [None, Some(10.0), Some(60.0)] {
        for margin_auto in [[false; 4], [true, false, false, true], [true; 4]] {
            for (margin, padding, border) in [(0.0, 0.0, 0.0), (7.0, 3.0, 2.0), (-5.0, 0.0, 1.0)] {
                for region in [5.0, 40.0, 100.0] {
                    let mut spec = fixed_margin_spec();
                    spec.width = size;
                    spec.height = size;
                    spec.margin_auto = margin_auto;
                    spec.margin = [margin; 4];
                    spec.padding = [padding; 4];
                    spec.border = [Some((border, CssColor::BLACK)); 4];
                    for axis in [Axis::Horizontal, Axis::Vertical] {
                        let sizes = [DroppedMargin::Start, DroppedMargin::End]
                            .map(|dropped| spec.fixed_extent(axis, region, dropped).content);
                        assert_eq!(
                            sizes[0], sizes[1],
                            "{size:?} {margin_auto:?} {margin} {padding} {border} {region}"
                        );
                        assert_eq!(spec.fixed_content_size(axis, region), sizes[0]);
                    }
                }
            }
        }
    }
}
