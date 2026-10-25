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
        for item in [
            "align-self:anchor-center",
            "align-self:safe anchor-center",
            "align-self:unsafe anchor-center",
        ] {
            let (_, top, _, _) = item_rect(container, &format!("{FITS};{item}"));
            assert_eq!(top, 20.0, "{container} | {item}");
        }
        for (anchor, center) in [
            ("anchor-center", "center"),
            ("safe anchor-center", "safe center"),
            ("unsafe anchor-center", "unsafe center"),
        ] {
            assert_eq!(
                item_rect(container, &format!("{TALL};align-self:{anchor}")),
                item_rect(container, &format!("{TALL};align-self:{center}")),
                "{container} | {anchor}"
            );
        }
    }
    let (left, _, _, _) = item_rect(GRID, &format!("{FITS};justify-self:anchor-center"));
    assert_eq!(left, 40.0);
    let (_, top, _, _) = item_rect(&format!("{ROW};align-items:anchor-center"), FITS);
    assert_eq!(top, 20.0, "align-items");
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
