//! `self-start`, `self-end` and the `safe` and `unsafe` overflow positions of
//! the self-alignment properties reach flex and grid layout as written, where
//! they used to fold into `start`, `end` and the default overflow alignment.

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// The border box `(x, y, width, height)` of an item styled by `item_css` in a
/// container styled by `container_css`, relative to the container.
fn item_rect(container_css: &str, item_css: &str) -> (f32, f32, f32, f32) {
    let mut item = 0;
    let fixture = block_fixture(container_css, |doc, root| {
        item = doc.append_element(
            Some(root),
            "div",
            Some(&format!("display:block;{item_css}")),
        );
    });
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (container_x, container_y, _, _) = absolute_rect(&doc, root);
    let (x, y, width, height) = absolute_rect(&doc, item);
    (x - container_x, y - container_y, width, height)
}

const ROW: &str = "display:flex;width:100px;height:50px;align-items:flex-start";
const COLUMN: &str = "display:flex;flex-direction:column;width:100px;height:50px";
const GRID: &str = "display:grid;grid-template:50px / 100px;justify-items:start;align-items:start";
const FITS: &str = "width:20px;height:10px";
const TALL: &str = "width:20px;height:80px";

#[test]
fn a_safe_alignment_that_fits_is_the_alignment() {
    for container in [ROW, GRID] {
        for (item, y) in [
            ("align-self:center", 20.0),
            ("align-self:safe center", 20.0),
            ("align-self:unsafe center", 20.0),
            ("align-self:end", 40.0),
            ("align-self:safe end", 40.0),
            ("align-self:safe flex-end", 40.0),
            ("align-self:safe start", 0.0),
        ] {
            let (_, top, _, _) = item_rect(container, &format!("{FITS};{item}"));
            assert_eq!(top, y, "{container} | {item}");
        }
    }
}

#[test]
fn a_safe_alignment_that_overflows_falls_back_to_the_start() {
    for container in [ROW, GRID] {
        for (item, y) in [
            ("align-self:safe center", 0.0),
            ("align-self:safe end", 0.0),
            ("align-self:safe flex-end", 0.0),
            ("align-self:unsafe center", -15.0),
            ("align-self:unsafe end", -30.0),
        ] {
            let (_, top, _, _) = item_rect(container, &format!("{TALL};{item}"));
            assert_eq!(top, y, "{container} | {item}");
        }
    }
}

#[test]
fn a_container_default_keeps_the_overflow_position_of_its_items() {
    let (_, top, _, _) = item_rect(&format!("{ROW};align-items:safe center"), TALL);
    assert_eq!(top, 0.0);
    let (_, top, _, _) = item_rect(&format!("{ROW};align-items:unsafe center"), TALL);
    assert_eq!(top, -15.0);
    let (_, top, _, _) = item_rect(&format!("{ROW};align-items:safe center"), FITS);
    assert_eq!(top, 20.0);
}

#[test]
fn self_start_and_self_end_follow_the_direction_of_the_item() {
    // The inline axis of a column flex container is its cross axis. A box
    // whose direction is `rtl` has its start edge at the right.
    let rtl = format!("{FITS};direction:rtl");
    for (item, x) in [
        ("align-self:start", 0.0),
        ("align-self:end", 80.0),
        ("align-self:self-start", 80.0),
        ("align-self:self-end", 0.0),
        ("align-self:safe self-start", 80.0),
        ("align-self:unsafe self-end", 0.0),
    ] {
        let (left, _, _, _) = item_rect(COLUMN, &format!("{rtl};{item}"));
        assert_eq!(left, x, "{item}");
    }
    // With the same direction as the container, they are `start` and `end`.
    for (item, x) in [
        ("align-self:self-start", 0.0),
        ("align-self:self-end", 80.0),
    ] {
        let (left, _, _, _) = item_rect(COLUMN, &format!("{FITS};{item}"));
        assert_eq!(left, x, "{item}");
    }
}

#[test]
fn self_start_and_self_end_of_a_grid_item_follow_its_direction() {
    let rtl = format!("{FITS};direction:rtl");
    for (item, x) in [
        ("justify-self:start", 0.0),
        ("justify-self:self-start", 80.0),
        ("justify-self:self-end", 0.0),
    ] {
        let (left, _, _, _) = item_rect(GRID, &format!("{rtl};{item}"));
        assert_eq!(left, x, "{item}");
    }
}

/// An absolutely positioned item that overflows its containing block aligned
/// by `safe center` or `safe end` falls back to `start`, the start edge of the
/// containing block's writing mode, wherever it is placed from.
#[test]
fn a_safe_static_position_that_overflows_falls_back_to_the_logical_start() {
    let flex =
        "display:flex;height:50px;width:50px;border:3px solid black;position:relative;margin:20px";
    let child = "position:absolute;border:2px dotted purple;width:65px;height:65px";
    let grid = "display:grid;padding:2px 1px;border:1px solid black;position:relative;width:30px;\
                height:25px;grid:3px 14px 3px / 2px 20px 2px";
    let placed = "position:absolute;grid-area:2 / 2 / 3 / 3;width:21px;height:21px";
    let mut cases = Vec::new();
    for mode in [
        "",
        ";writing-mode:vertical-rl",
        ";writing-mode:vertical-lr",
        ";writing-mode:vertical-rl;direction:rtl",
    ] {
        for direction in ["row", "column", "row-reverse", "column-reverse"] {
            let container = format!("{flex};flex-direction:{direction}{mode}");
            cases.push((container.clone(), child.to_string(), ("", "")));
            cases.push((container, format!("{child};margin:5px"), ("", "")));
        }
        cases.push((
            format!("{grid}{mode}"),
            placed.to_string(),
            (";justify-self:start", ";justify-self:safe end"),
        ));
    }
    let mut mismatches = Vec::new();
    for (container, item, (start_justify, safe_justify)) in cases {
        for safe in ["safe center", "safe end", "safe start"] {
            let expected = item_rect(
                &container,
                &format!("{item};align-self:start{start_justify}"),
            );
            let actual = item_rect(
                &container,
                &format!("{item};align-self:{safe}{safe_justify}"),
            );
            if actual != expected {
                mismatches.push(format!(
                    "{container} | {item} | {safe}: {actual:?} instead of {expected:?}"
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A `safe` static alignment of an item that fits is the alignment, in every
/// writing mode and direction.
#[test]
fn a_safe_static_position_that_fits_is_the_alignment() {
    let flex = "display:flex;height:50px;width:50px;position:relative";
    let child = "position:absolute;width:20px;height:10px";
    for mode in ["", ";writing-mode:vertical-rl", ";writing-mode:vertical-lr"] {
        for direction in ["", ";direction:rtl"] {
            for align in ["center", "start", "end"] {
                let container = format!("{flex}{mode}{direction}");
                assert_eq!(
                    item_rect(&container, &format!("{child};align-self:safe {align}")),
                    item_rect(&container, &format!("{child};align-self:{align}")),
                    "{container} | {align}"
                );
            }
        }
    }
}

/// Where only the axis that runs from right to left aligns `safe`, the other
/// axis, which runs from bottom to top, keeps its alignment.
#[test]
fn a_safe_static_position_in_one_axis_leaves_the_other_alone() {
    let container = "display:flex;height:50px;width:50px;position:relative;\
                     writing-mode:vertical-rl;direction:rtl;justify-content:center";
    let child = "position:absolute;width:65px;height:65px";
    assert_eq!(
        item_rect(container, &format!("{child};align-self:safe center")),
        item_rect(container, &format!("{child};align-self:start")),
    );
}

/// The static position of a `safe` alignment is resolved in the area it is
/// aligned in, which is the content box of a flex container: the layout in
/// `vertical-rl` is the mirror image of the one in `vertical-lr`.
#[test]
fn a_safe_static_position_in_vertical_rl_mirrors_vertical_lr() {
    let flex = "display:flex;height:50px;width:50px;border:3px solid black;position:relative";
    for (left, right) in [(0, 0), (20, 0), (0, 20), (30, 5)] {
        let width = 50.0 + 6.0 + (left + right) as f32;
        for size in [10, 65] {
            for align in ["safe center", "safe start", "safe end"] {
                let child =
                    format!("position:absolute;width:{size}px;height:{size}px;align-self:{align}");
                let (lr_left, lr_top, lr_width, lr_height) = item_rect(
                    &format!("{flex};writing-mode:vertical-lr;padding:0 {right}px 0 {left}px"),
                    &child,
                );
                let rl = item_rect(
                    &format!("{flex};writing-mode:vertical-rl;padding:0 {left}px 0 {right}px"),
                    &child,
                );
                assert_eq!(
                    rl,
                    (width - lr_left - lr_width, lr_top, lr_width, lr_height),
                    "{left} {right} | {size} | {align}"
                );
            }
        }
    }
}

/// A `last baseline` alignment, which falls back to `safe self-end`, keeps an
/// overflowing box at the end edge of its grid area in a vertical grid, as the
/// web-platform-tests expect, where an explicit `safe end` falls back to the
/// start edge.
#[test]
fn a_last_baseline_static_position_keeps_the_end_edge_when_it_overflows() {
    let grid = "display:grid;padding:2px 1px;border:1px solid black;position:relative;width:2px;\
                height:4px;grid:0px 2px 0px / 3px 2px 3px;writing-mode:vertical-rl";
    let child = "position:absolute;grid-area:2 / 2 / 3 / 3;width:6px;height:8px";
    let end = item_rect(grid, &format!("{child};align-self:end"));
    assert_eq!(
        item_rect(grid, &format!("{child};align-self:last baseline")),
        end
    );
    assert_ne!(
        item_rect(grid, &format!("{child};align-self:safe end")),
        end
    );
}

/// The last-baseline exception applies to the axis that aligns to the last
/// baseline only: an explicit `safe` alignment in the other axis still falls
/// back to the start of that axis.
#[test]
fn a_last_baseline_in_one_axis_leaves_the_safe_alignment_of_the_other() {
    let grid = "display:grid;padding:2px 1px;border:1px solid black;position:relative;width:30px;\
                height:25px;grid:3px 14px 3px / 2px 20px 2px";
    let child = "position:absolute;grid-area:2 / 2 / 3 / 3;width:21px;height:21px";
    // The block axis, which runs from right to left, aligns `safe`.
    let container = format!("{grid};writing-mode:vertical-rl");
    assert_eq!(
        item_rect(
            &container,
            &format!("{child};align-self:safe center;justify-self:last baseline")
        ),
        item_rect(
            &container,
            &format!("{child};align-self:start;justify-self:last baseline")
        ),
    );
    // The inline axis, which runs from bottom to top, aligns `safe`.
    let container = format!("{container};direction:rtl");
    assert_eq!(
        item_rect(
            &container,
            &format!("{child};align-self:last baseline;justify-self:safe end")
        ),
        item_rect(
            &container,
            &format!("{child};align-self:last baseline;justify-self:start")
        ),
    );
}

const BLOCK: &str = "position:relative;width:100px;height:100px";
const ABSOLUTE: &str = "position:absolute;width:50px;height:50px";

#[test]
fn anchor_center_aligns_an_in_flow_item_like_center() {
    for container in [ROW, GRID] {
        let (_, top, _, _) = item_rect(container, &format!("{FITS};align-self:anchor-center"));
        assert_eq!(top, 20.0, "{container}");
        assert_eq!(
            item_rect(container, &format!("{TALL};align-self:anchor-center")),
            item_rect(container, &format!("{TALL};align-self:center")),
            "{container}"
        );
    }
    let (left, _, _, _) = item_rect(GRID, &format!("{FITS};justify-self:anchor-center"));
    assert_eq!(left, 40.0);
}

#[test]
fn anchor_center_is_ignored_where_it_is_not_a_value() {
    // `align-items` does not take it, so the declaration is dropped and the
    // container keeps its `flex-start` default; `safe anchor-center` is
    // invalid as well and leaves the earlier `align-self` alone.
    let (_, top, _, _) = item_rect(&format!("{ROW};align-items:anchor-center"), FITS);
    assert_eq!(top, 0.0);
    let (_, top, _, _) = item_rect(
        ROW,
        &format!("{FITS};align-self:end;align-self:safe anchor-center"),
    );
    assert_eq!(top, 40.0);
}

#[test]
fn anchor_center_places_an_absolute_box_without_an_anchor_like_center() {
    for axis in ["align-self", "justify-self"] {
        let center = item_rect(BLOCK, &format!("{ABSOLUTE};{axis}:center"));
        let anchor = item_rect(BLOCK, &format!("{ABSOLUTE};{axis}:anchor-center"));
        let normal = item_rect(BLOCK, &format!("{ABSOLUTE};{axis}:normal"));
        assert_eq!(anchor, center, "{axis}");
        assert_ne!(anchor, normal, "{axis}: the keyword moves the box");
    }
}

/// An absolutely positioned box of a block container that asks for `last
/// baseline` sits where `end` puts it in the static-position rectangle, a line
/// of no height in the block axis, wherever it overflows the rectangle; `safe
/// end` falls back to the start there. The end is the one of the container, not
/// of the box.
#[test]
fn a_last_baseline_static_position_in_a_block_aligns_to_the_end() {
    for (axis, fits, overflows) in [
        // The static position is the top edge of the container in the block
        // axis and the whole content box in the inline axis.
        ("align-self", (0.0, -50.0), (0.0, -150.0)),
        ("justify-self", (50.0, 0.0), (-50.0, 0.0)),
    ] {
        for (size, (x, y)) in [("", fits), (";width:150px;height:150px", overflows)] {
            let child = format!("{ABSOLUTE}{size}");
            let (left, top, _, _) = item_rect(BLOCK, &format!("{child};{axis}:last baseline"));
            assert_eq!((left, top), (x, y), "{axis} {size}");
            let end = item_rect(BLOCK, &format!("{child};{axis}:end"));
            assert_eq!((end.0, end.1), (x, y), "{axis} {size}");
        }
    }
    let overflowing = format!("{ABSOLUTE};width:150px;height:150px");
    let (left, _, _, _) = item_rect(BLOCK, &format!("{overflowing};justify-self:safe end"));
    assert_eq!(left, 0.0);
    let (_, top, _, _) = item_rect(BLOCK, &format!("{overflowing};align-self:safe end"));
    assert_eq!(top, 0.0);
}

#[test]
fn a_last_baseline_static_position_in_a_block_follows_the_direction_of_the_container() {
    let rtl = format!("{BLOCK};direction:rtl");
    let child = format!("{ABSOLUTE};direction:ltr");
    // `end` is the left edge of an `rtl` container; `self-end` is the right edge
    // of the `ltr` box.
    for (item, left) in [
        ("justify-self:last baseline", 0.0),
        ("justify-self:end", 0.0),
        ("justify-self:self-end", 50.0),
    ] {
        let (x, _, _, _) = item_rect(&rtl, &format!("{child};{item}"));
        assert_eq!(x, left, "{item}");
    }
}

/// The same holds in every writing mode and direction of the container, where
/// `safe end` falls back to the start in the block axis of a horizontal
/// container and in the inline axis of a vertical one.
#[test]
fn a_last_baseline_static_position_in_a_block_is_the_end_in_every_writing_mode() {
    for writing_mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for direction in ["ltr", "rtl"] {
            let container = format!("{BLOCK};writing-mode:{writing_mode};direction:{direction}");
            for axis in ["align-self", "justify-self"] {
                assert_eq!(
                    item_rect(&container, &format!("{ABSOLUTE};{axis}:last baseline")),
                    item_rect(&container, &format!("{ABSOLUTE};{axis}:end")),
                    "{writing_mode} {direction} {axis}"
                );
            }
        }
    }
    let vertical = format!("{BLOCK};writing-mode:vertical-rl");
    assert_ne!(
        item_rect(&vertical, &format!("{ABSOLUTE};align-self:last baseline")),
        item_rect(&vertical, &format!("{ABSOLUTE};align-self:safe end")),
    );
}

/// A `last baseline` alignment in one axis reads on its own: the other axis
/// keeps the alignment it has, and an inset in the axis settles the position
/// there as it does for every alignment.
#[test]
fn a_last_baseline_static_position_in_a_block_leaves_the_other_axis_alone() {
    let (left, top, _, _) = item_rect(
        BLOCK,
        &format!("{ABSOLUTE};align-self:last baseline;justify-self:center"),
    );
    assert_eq!((left, top), (25.0, -50.0));
    let (left, top, _, _) = item_rect(
        BLOCK,
        &format!("{ABSOLUTE};align-self:last baseline;top:10px"),
    );
    assert_eq!((left, top), (0.0, 10.0));
}

#[test]
fn an_overflowing_flex_item_keeps_its_cross_size() {
    for mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for (container, y) in [
            ("height:80px;align-items:flex-end", -90.0),
            ("height:80px;align-items:center", -45.0),
            ("height:80px;align-items:flex-start", 0.0),
            ("height:80px;align-items:stretch", 0.0),
            ("height:80px;align-items:flex-end;flex-wrap:wrap", 0.0),
            ("min-height:80px;align-items:flex-end", 0.0),
        ] {
            for sibling in [false, true] {
                for text in ["", "x"] {
                    let mut item = 0;
                    let fixture = block_fixture(
                        &format!("display:flex;width:100px;{container}"),
                        |doc, root| {
                            if sibling {
                                doc.append_element(
                                    Some(root),
                                    "div",
                                    Some("display:block;width:10px;height:50px"),
                                );
                            }
                            item = doc.append_element(
                                Some(root),
                                "div",
                                Some(&format!(
                                    "display:block;writing-mode:{mode};width:10px;height:170px"
                                )),
                            );
                            doc.append_text(item, text);
                        },
                    );
                    let root = fixture.root;
                    let mut doc = fixture.doc;
                    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600())
                        .expect("layout");
                    let (root_x, root_y, _, _) = absolute_rect(&doc, root);
                    let (x, top, width, height) = absolute_rect(&doc, item);
                    assert_eq!(
                        (x - root_x, top - root_y, width, height),
                        (if sibling { 10.0 } else { 0.0 }, y, 10.0, 170.0),
                        "{mode} | {container} | sibling={sibling} | {text}"
                    );
                }
            }
        }
    }
}

/// The border box of the box styled by `item_css` inside a wrapper styled by
/// `wrapper_css` inside a container styled by `container_css`, relative to the
/// container.
fn wrapped_item_rect(
    container_css: &str,
    wrapper_css: &str,
    item_css: &str,
) -> (f32, f32, f32, f32) {
    let mut item = 0;
    let fixture = block_fixture(container_css, |doc, root| {
        let wrapper = doc.append_element(Some(root), "div", Some(wrapper_css));
        item = doc.append_element(
            Some(wrapper),
            "div",
            Some(&format!("display:block;{item_css}")),
        );
    });
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (container_x, container_y, _, _) = absolute_rect(&doc, root);
    let (x, y, width, height) = absolute_rect(&doc, item);
    (x - container_x, y - container_y, width, height)
}

const SQUARE_GRID: &str = "display:grid;width:200px;height:200px;grid-template:200px / 200px";
const SMALL: &str = "width:50px;height:20px";

#[test]
fn left_and_right_are_physical_edges_of_a_grid_area() {
    for direction in ["ltr", "rtl"] {
        for item_direction in ["", "direction:ltr;", "direction:rtl;"] {
            for (alignment, x) in [
                ("left", 0.0),
                ("safe left", 0.0),
                ("unsafe left", 0.0),
                ("right", 150.0),
                ("safe right", 150.0),
                ("unsafe right", 150.0),
            ] {
                let container = format!("{SQUARE_GRID};direction:{direction}");
                let (left, ..) = item_rect(
                    &container,
                    &format!("{SMALL};{item_direction}justify-self:{alignment}"),
                );
                assert_eq!(left, x, "{direction} | {item_direction}{alignment}");
                let (left, ..) =
                    item_rect(&format!("{container};justify-items:{alignment}"), SMALL);
                assert_eq!(left, x, "{direction} | justify-items:{alignment}");
            }
        }
    }
}

#[test]
fn left_is_the_line_left_edge_of_a_vertical_grid_area() {
    // Line-left is the top in both vertical-rl and vertical-lr, and the bottom
    // in sideways-lr; a right-to-left grid starts its lines at the other end.
    for (mode, direction, left_y, right_y) in [
        ("vertical-rl", "ltr", 0.0, 180.0),
        ("vertical-rl", "rtl", 0.0, 180.0),
        ("vertical-lr", "rtl", 0.0, 180.0),
        ("sideways-lr", "ltr", 180.0, 0.0),
        ("sideways-lr", "rtl", 180.0, 0.0),
    ] {
        for (property, container_extra) in [("justify-self", false), ("justify-items", true)] {
            let container = format!("{SQUARE_GRID};writing-mode:{mode};direction:{direction}");
            for (alignment, y) in [("left", left_y), ("right", right_y)] {
                let (_, top, _, _) = if container_extra {
                    item_rect(&format!("{container};{property}:{alignment}"), SMALL)
                } else {
                    item_rect(&container, &format!("{SMALL};{property}:{alignment}"))
                };
                assert_eq!(top, y, "{mode} {direction} | {property}:{alignment}");
            }
        }
    }
}

#[test]
fn left_follows_the_direction_of_the_alignment_container() {
    // A `display: contents` wrapper makes no box: the grid is the container.
    for direction in ["ltr", "rtl"] {
        let other = if direction == "rtl" { "ltr" } else { "rtl" };
        for (alignment, x) in [("left", 0.0), ("right", 150.0)] {
            let (left, ..) = wrapped_item_rect(
                &format!("{SQUARE_GRID};direction:{direction}"),
                &format!("display:contents;direction:{other}"),
                &format!("{SMALL};justify-self:{alignment}"),
            );
            assert_eq!(left, x, "contents {direction} | {alignment}");
            // The containing block of an absolutely positioned box is its
            // alignment container, not the parent between them.
            for container in [
                "position:relative;width:200px;height:200px",
                "display:grid;position:relative;width:200px;height:200px",
            ] {
                let (left, ..) = wrapped_item_rect(
                    &format!("{container};direction:{direction}"),
                    &format!("direction:{other}"),
                    &format!("position:absolute;inset:0;{SMALL};justify-self:{alignment}"),
                );
                assert_eq!(left, x, "{container} {direction} | abspos {alignment}");
            }
        }
    }
}

#[test]
fn left_in_a_static_position_follows_the_box_that_holds_it() {
    for (direction, other) in [("ltr", "rtl"), ("rtl", "ltr")] {
        // A grid aligns its static-position area with its own direction.
        for (alignment, x) in [("left", 0.0), ("right", 150.0)] {
            let (left, ..) = wrapped_item_rect(
                &format!(
                    "position:relative;display:grid;width:200px;height:200px;direction:{direction}"
                ),
                &format!("display:grid;width:200px;direction:{other}"),
                &format!("position:absolute;{SMALL};justify-self:{alignment}"),
            );
            assert_eq!(left, x, "grid {direction} | {alignment}");
        }
        // A block container reads them as its start and end.
        let (start_x, end_x) = if other == "rtl" {
            (150.0, 0.0)
        } else {
            (0.0, 150.0)
        };
        for (alignment, x) in [("left", start_x), ("right", end_x)] {
            let (left, ..) = wrapped_item_rect(
                &format!("position:relative;width:200px;height:200px;direction:{direction}"),
                &format!("width:200px;direction:{other}"),
                &format!("position:absolute;{SMALL};justify-self:{alignment}"),
            );
            assert_eq!(left, x, "block {direction} | {alignment}");
        }
    }
}

#[test]
fn right_keeps_both_insets_of_a_vertical_keyword_height_box() {
    // In a vertical containing block, `justify-self` aligns along the height:
    // `right` (line-right, the bottom) keeps both insets and ends at the
    // bottom, `left` drops the bottom one.
    for direction in ["ltr", "rtl"] {
        for (alignment, bottom_aligned) in [("left", false), ("right", true)] {
            let (_, top, _, height) = item_rect(
                &format!(
                    "position:relative;width:200px;height:200px;writing-mode:vertical-rl;direction:{direction}"
                ),
                &format!(
                    "position:absolute;top:0;bottom:0;width:50px;height:max-content;justify-self:{alignment}"
                ),
            );
            let y = if bottom_aligned { 200.0 - height } else { 0.0 };
            assert_eq!(top, y, "{direction} | {alignment}");
        }
    }
}
