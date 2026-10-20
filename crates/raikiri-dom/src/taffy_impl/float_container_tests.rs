//! A floated flex or grid container is shrink-to-fit (CSS 2.1 §10.3.5): its
//! width is its max-content width, within the room its parent leaves and at
//! least its min-content width. The text of the fixtures is `A A` in Ahem at
//! 10px: 10px wide at its longest word, 30px wide on one line, in an item with
//! 5px margins, 3px padding and a 2px border.

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// A border box `(x, y, width, height)`.
type BorderBox = (f32, f32, f32, f32);

/// The border box of the container `container_css` styles and of its only
/// item, in a parent styled by `parent_css`.
fn laid_out(parent_css: &str, container_css: &str, item_css: &str) -> (BorderBox, BorderBox) {
    laid_out_beside(parent_css, container_css, item_css, &[])
}

/// [`laid_out`] with an empty sibling of the item for each entry of
/// `siblings`, styled by it.
fn laid_out_beside(
    parent_css: &str,
    container_css: &str,
    item_css: &str,
    siblings: &[&str],
) -> (BorderBox, BorderBox) {
    let mut container = 0;
    let mut item = 0;
    let fixture = block_fixture(&format!("line-height:10px;{parent_css}"), |doc, root| {
        container = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some(&format!("float:left;{container_css}")),
        );
        item = doc.append_element(
            Some(container),
            "div",
            crate::LayoutStyle::default(),
            Some(&format!(
                "display:block;margin:5px;padding:3px;border:2px solid;{item_css}"
            )),
        );
        doc.append_text(item, "A A");
        for &sibling in siblings {
            doc.append_element(
                Some(container),
                "div",
                crate::LayoutStyle::default(),
                Some(sibling),
            );
        }
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    (absolute_rect(&doc, container), absolute_rect(&doc, item))
}

/// The border box width of the container, to the hundredth of a pixel.
fn container_width(parent_css: &str, container_css: &str, item_css: &str) -> f32 {
    hundredths(laid_out(parent_css, container_css, item_css).0.2)
}

fn hundredths(size: f32) -> f32 {
    (size * 100.0).round() / 100.0
}

const FLEX: &str = "display:flex";
const GRID: &str = "display:grid";

#[test]
fn a_floated_flex_or_grid_container_takes_the_max_content_width_in_room_enough() {
    for container in [FLEX, GRID] {
        assert_eq!(container_width("", container, ""), 50.0, "{container}");
    }
}

#[test]
fn a_floated_flex_container_measures_its_items_content_not_their_flex_basis() {
    // A flexible item whose flex basis is wider than its content shrinks to the
    // content in a shrink-to-fit container.
    assert_eq!(container_width("", FLEX, "flex:1 1 60px"), 50.0);
    assert_eq!(container_width("", FLEX, "flex:0 1 60px"), 50.0);
}

#[test]
fn a_floated_flex_or_grid_container_is_no_narrower_than_its_min_content_width() {
    for container in [FLEX, GRID] {
        // The room is narrower than the longest word and its edges.
        assert_eq!(
            container_width("width:20px", container, ""),
            30.0,
            "{container}"
        );
    }
}

#[test]
fn a_floated_flex_or_grid_container_takes_the_room_between_its_intrinsic_widths() {
    for container in [FLEX, GRID] {
        assert_eq!(
            container_width("width:40px", container, ""),
            40.0,
            "{container}"
        );
    }
}

#[test]
fn a_floated_flex_or_grid_container_with_a_keyword_width_is_sized_by_the_keyword() {
    for container in [FLEX, GRID] {
        for (width, expected) in [
            ("max-content", 50.0),
            ("min-content", 30.0),
            ("fit-content", 50.0),
        ] {
            assert_eq!(
                container_width("", &format!("{container};width:{width}"), "flex:1 1 60px"),
                expected,
                "{container} {width}"
            );
        }
        // The room the parent leaves does not limit a `max-content` width.
        assert_eq!(
            container_width(
                "width:40px",
                &format!("{container};width:max-content"),
                "flex:1 1 60px"
            ),
            50.0,
            "{container}"
        );
    }
}

#[test]
fn a_floated_flex_or_grid_container_with_a_length_width_keeps_it() {
    for container in [FLEX, GRID] {
        assert_eq!(
            container_width("", &format!("{container};width:120px"), ""),
            120.0,
            "{container}"
        );
    }
}

#[test]
fn a_floated_flex_or_grid_container_measures_the_keyword_bounds_of_its_items() {
    // The item is 10px wide, but its minimum width is the max-content width of
    // its text, so the container is as wide as the item at that minimum.
    for container in [FLEX, GRID] {
        assert_eq!(
            container_width("", container, "width:10px;min-width:max-content"),
            50.0,
            "{container}"
        );
    }
}

#[test]
fn a_floated_flex_container_is_laid_out_at_the_width_it_shrinks_to() {
    let (container, item) = laid_out("", FLEX, "flex:1 1 60px");
    assert_eq!(container, (0.0, 0.0, 50.0, 30.0));
    assert_eq!(item, (5.0, 5.0, 40.0, 20.0));
}

#[test]
fn a_floated_flex_container_is_at_least_as_wide_as_its_items_that_cannot_shrink() {
    // The item keeps its 100px flex base size, so the container does not shrink
    // below it however little room its parent leaves. The item's margin, padding
    // and border come on top of a content-box flex basis.
    assert_eq!(container_width("width:0", FLEX, "flex:1 0 100px"), 120.0);
    assert_eq!(
        container_width("width:0", FLEX, "flex:1 0 100px;box-sizing:border-box"),
        110.0
    );
}

#[test]
fn a_floated_flex_container_is_no_wider_than_the_flex_basis_of_items_that_cannot_grow() {
    // The 100px basis is a ceiling for the item, which would otherwise measure
    // its 30px of text.
    assert_eq!(container_width("", FLEX, "flex:0 1 100px"), 50.0);
    assert_eq!(
        container_width("", FLEX, "flex:0 1 20px;min-width:0;margin:0"),
        30.0
    );
    // The margins stay on top of the capped item: 20px of basis, 10px of
    // padding and border, 10px of margin.
    assert_eq!(container_width("", FLEX, "flex:0 1 20px;min-width:0"), 40.0);
}

#[test]
fn a_floated_flex_container_ignores_a_percentage_bound_of_its_items() {
    // The percentage refers to the container's width, which the container is
    // still deriving from its items, so the item's width is not bounded by it.
    assert_eq!(
        container_width("", FLEX, "flex:1 0 100px;min-width:10%"),
        120.0
    );
    assert_eq!(
        container_width("", FLEX, "flex:0 1 20px;max-width:10%;margin:0"),
        30.0
    );
    assert_eq!(
        container_width("", FLEX, "flex:0 1 20px;max-width:10%;min-width:0"),
        40.0
    );
}

#[test]
fn a_floated_column_flex_container_is_not_widened_by_the_flex_basis_of_its_items() {
    // The flex basis is a height there.
    assert_eq!(
        container_width("", "display:flex;flex-direction:column", "flex:1 0 100px"),
        50.0
    );
}

/// The border box width of a floated container with `count` items 30px wide
/// plus a 1px border, styled by `container_css` after a 1px border and a 12px
/// height.
fn floated_width(container_css: &str, count: usize) -> f32 {
    let mut container = 0;
    let fixture = block_fixture("line-height:10px", |doc, root| {
        container = doc.append_element(
            Some(root),
            "div",
            crate::LayoutStyle::default(),
            Some(&format!(
                "float:left;border:1px solid;height:12px;{container_css}"
            )),
        );
        for _ in 0..count {
            doc.append_element(
                Some(container),
                "div",
                crate::LayoutStyle::default(),
                Some("display:block;width:30px;border:1px solid"),
            );
        }
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    hundredths(absolute_rect(&doc, container).2)
}

/// [`floated_width`] of a wrapping flex container.
fn wrapping_width(container_css: &str, count: usize) -> f32 {
    floated_width(
        &format!("display:flex;flex-wrap:wrap;{container_css}"),
        count,
    )
}

#[test]
fn a_floated_wrapping_flex_container_is_measured_as_a_single_line() {
    // Five 32px items on one line, plus the container's 2px of border.
    assert_eq!(wrapping_width("", 5), 162.0);
    // The maximum width clamps the single line, it does not wrap it first.
    assert_eq!(wrapping_width("max-width:120px", 5), 122.0);
    assert_eq!(wrapping_width("max-width:120px;min-width:100px", 5), 122.0);
    // The minimum width wins over the maximum width.
    assert_eq!(wrapping_width("min-width:150px;max-width:120px", 5), 152.0);
    assert_eq!(wrapping_width("min-width:100px", 0), 102.0);
    assert_eq!(wrapping_width("min-width:100px", 1), 102.0);
}

#[test]
fn a_floated_empty_grid_container_is_as_wide_as_its_explicit_tracks() {
    assert_eq!(
        floated_width("display:grid;grid-template-columns:100px", 0),
        102.0
    );
    assert_eq!(
        floated_width("display:grid;grid-template-columns:100px 50px", 0),
        152.0
    );
}

#[test]
fn a_floated_flex_container_keeps_the_flex_basis_floor_under_a_keyword_minimum() {
    // The minimum width is the 30px of text, below the 100px an item that
    // cannot shrink keeps.
    assert_eq!(
        container_width("", FLEX, "flex:1 0 100px;min-width:max-content"),
        120.0
    );
    assert_eq!(
        container_width("width:0", FLEX, "flex:1 0 100px;min-width:max-content"),
        120.0
    );
    // A minimum above the basis wins.
    assert_eq!(
        container_width("", FLEX, "flex:1 0 20px;min-width:max-content"),
        50.0
    );
}

#[test]
fn a_floated_flex_container_bounds_an_explicitly_sized_item_by_its_flex_basis() {
    // The flex basis wins over the width for the flex base size, so the item
    // that cannot shrink keeps 100px and the one that cannot grow stops at 20px.
    assert_eq!(
        container_width("width:0", FLEX, "flex:1 0 100px;width:20px"),
        120.0
    );
    assert_eq!(
        container_width("", FLEX, "flex:0 1 20px;width:200px;min-width:0;margin:0"),
        30.0
    );
    // A width above the basis of an item that cannot shrink is its size.
    assert_eq!(
        container_width("width:0", FLEX, "flex:1 0 100px;width:200px"),
        220.0
    );
    // The margins of an explicitly sized item stay on top of the capped width.
    assert_eq!(
        container_width("", FLEX, "flex:0 1 20px;width:200px;min-width:0"),
        40.0
    );
    // An item that can neither shrink nor grow is as wide as its basis.
    assert_eq!(
        container_width("width:0", FLEX, "flex:0 0 100px;width:200px"),
        120.0
    );
    assert_eq!(
        container_width("width:0", FLEX, "flex:0 0 100px;width:20px"),
        120.0
    );
}

#[test]
fn a_floated_flex_container_is_not_widened_by_the_flex_basis_of_boxes_outside_its_flow() {
    // Neither a `display: none` child nor an absolutely positioned one is a
    // flex item, so their bases are not floors for the container.
    let (container, _) = laid_out_beside(
        "width:0",
        FLEX,
        "",
        &[
            "display:none;flex:1 0 100px",
            "position:absolute;flex:1 0 100px",
        ],
    );
    // No room: the min-content width of the item, 10px of text and its edges.
    assert_eq!(hundredths(container.2), 30.0);
}

#[test]
fn a_floated_flex_container_is_not_floored_by_a_content_based_flex_basis() {
    // Only a flex basis that is a length floors the min-content width of the
    // container by the flex base size of an item that cannot shrink. Browsers
    // leave the item's own min-content contribution where the flex basis is
    // content-based (`auto`, `max-content`, a percentage), so the container
    // takes the 10px of its longest word and the item overflows it, as in
    // css-flexbox/intrinsic-size/row-001.
    for item in [
        "flex:none",
        "flex:0 0 auto",
        "flex:1 0 max-content",
        "flex:0 0 50%",
    ] {
        assert_eq!(container_width("width:0", FLEX, item), 30.0, "{item}");
    }
    // The `width` is the item's contribution, and a content-based flex basis
    // does not raise it, whether the parent leaves room or not.
    for item in [
        "flex:0 0 auto;width:10px",
        "flex:1 0 max-content;width:10px",
        "flex:0 0 50%;width:10px",
    ] {
        assert_eq!(container_width("width:0", FLEX, item), 30.0, "{item}");
    }
    assert_eq!(
        container_width("", FLEX, "flex:1 0 max-content;width:10px"),
        30.0
    );
}

#[test]
fn a_floated_flex_container_clamps_the_flex_basis_floor_by_the_bounds_of_its_items() {
    // The flex base size floors the contribution, and then the item's own
    // `max-width` caps it: 20px of width, 10px of padding and border, 10px of
    // margin.
    assert_eq!(
        container_width("width:0", FLEX, "flex:1 0 100px;max-width:20px"),
        40.0
    );
    assert_eq!(
        container_width("width:0", FLEX, "flex:1 0 100px;width:30px;max-width:20px"),
        40.0
    );
    // The `min-width` wins over the `max-width`.
    assert_eq!(
        container_width(
            "width:0",
            FLEX,
            "flex:1 0 100px;max-width:20px;min-width:50px"
        ),
        70.0
    );
}
