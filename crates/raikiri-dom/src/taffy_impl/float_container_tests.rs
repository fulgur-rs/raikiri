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
    let mut container = 0;
    let mut item = 0;
    let fixture = block_fixture(&format!("line-height:10px;{parent_css}"), |doc, root| {
        container = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some(&format!("float:left;{container_css}")),
        );
        item = doc.append_element(
            Some(container),
            "div",
            taffy::Style::default(),
            Some(&format!(
                "display:block;margin:5px;padding:3px;border:2px solid;{item_css}"
            )),
        );
        doc.append_text(item, "A A");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    (absolute_rect(&doc, container), absolute_rect(&doc, item))
}

fn container_width(parent_css: &str, container_css: &str, item_css: &str) -> f32 {
    laid_out(parent_css, container_css, item_css).0.2
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
fn a_floated_flex_container_is_laid_out_at_the_width_it_shrinks_to() {
    let (container, item) = laid_out("", FLEX, "flex:1 1 60px");
    assert_eq!(container, (0.0, 0.0, 50.0, 30.0));
    assert_eq!(item, (5.0, 5.0, 40.0, 20.0));
}
