//! `width` set to `min-content`, `max-content` or `fit-content` on a flex or
//! grid item takes the size the keyword measures (CSS Sizing 3 §3.1.1), and the
//! item is not stretched. The text of the fixtures is `AAAA BBBB` in Ahem at
//! 10px: 40px wide at its longest word, 90px wide on one line.

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// What the item holds.
#[derive(Clone, Copy)]
enum Body {
    /// The text, so the item is a paragraph root.
    Text,
    /// One block of the given width and 10px height.
    Block(f32),
}

/// The border box `(x, y, width, height)` of the item `item_css` styles in a
/// container styled by `container_css`.
fn item_rect(container_css: &str, item_css: &str, body: Body) -> (f32, f32, f32, f32) {
    let mut item = 0;
    let fixture = block_fixture(&format!("line-height:10px;{container_css}"), |doc, root| {
        item = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some(&format!("display:block;{item_css}")),
        );
        match body {
            Body::Text => {
                doc.append_text(item, "AAAA BBBB");
            }
            Body::Block(width) => {
                doc.append_element(
                    Some(item),
                    "div",
                    taffy::Style::default(),
                    Some(&format!("display:block;width:{width}px;height:10px")),
                );
            }
        }
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    absolute_rect(&doc, item)
}

fn size(container_css: &str, item_css: &str, body: Body) -> (f32, f32) {
    let (_, _, width, height) = item_rect(container_css, item_css, body);
    (width, height)
}

const COLUMN: &str = "display:flex;flex-direction:column;width:200px";
const NARROW_COLUMN: &str = "display:flex;flex-direction:column;width:50px";
const ROW: &str = "display:flex;width:200px";
const NARROW_ROW: &str = "display:flex;width:50px";
const GRID: &str = "display:grid;width:200px";
const NARROW_GRID: &str = "display:grid;width:50px";

#[test]
fn an_auto_width_still_stretches_a_column_flex_item() {
    assert_eq!(size(COLUMN, "width:auto", Body::Text), (200.0, 10.0));
}

#[test]
fn a_keyword_width_sizes_a_column_flex_item_instead_of_stretching_it() {
    // Room for the whole line: `fit-content` is the max-content width.
    assert_eq!(size(COLUMN, "width:max-content", Body::Text), (90.0, 10.0));
    assert_eq!(size(COLUMN, "width:fit-content", Body::Text), (90.0, 10.0));
    assert_eq!(
        size(COLUMN, "width:fit-content(60px)", Body::Text),
        (60.0, 20.0)
    );
    // The height follows the width the keyword gives: the longest word wraps
    // the line in two.
    assert_eq!(size(COLUMN, "width:min-content", Body::Text), (40.0, 20.0));
    assert_eq!(
        size(COLUMN, "width:max-content", Body::Block(100.0)),
        (100.0, 10.0)
    );
}

#[test]
fn a_column_flex_item_is_measured_at_the_width_of_its_keyword() {
    // `max-content` ignores the room, so the line does not wrap, and the item
    // overflows; `fit-content` takes the room.
    assert_eq!(
        size(NARROW_COLUMN, "width:max-content", Body::Text),
        (90.0, 10.0)
    );
    assert_eq!(
        size(NARROW_COLUMN, "width:fit-content", Body::Text),
        (50.0, 20.0)
    );
}

#[test]
fn a_keyword_width_aligns_a_column_flex_item_like_any_unstretched_item() {
    assert_eq!(
        item_rect(COLUMN, "width:max-content", Body::Text),
        (0.0, 0.0, 90.0, 10.0)
    );
    assert_eq!(
        item_rect(COLUMN, "width:max-content;align-self:center", Body::Text),
        (55.0, 0.0, 90.0, 10.0)
    );
    assert_eq!(
        item_rect(COLUMN, "width:max-content;align-self:flex-end", Body::Text),
        (110.0, 0.0, 90.0, 10.0)
    );
    assert_eq!(
        item_rect(COLUMN, "width:max-content;margin-left:auto", Body::Text),
        (110.0, 0.0, 90.0, 10.0)
    );
    // The container's own alignment applies as well.
    assert_eq!(
        item_rect(
            &format!("{COLUMN};align-items:center"),
            "width:max-content",
            Body::Text
        ),
        (55.0, 0.0, 90.0, 10.0)
    );
}

#[test]
fn a_keyword_width_sizes_a_row_flex_item_along_the_main_axis() {
    assert_eq!(size(ROW, "width:max-content", Body::Text), (90.0, 10.0));
    assert_eq!(size(ROW, "width:fit-content", Body::Text), (90.0, 10.0));
    assert_eq!(size(ROW, "width:min-content", Body::Text), (40.0, 20.0));
    // The item shrinks with the container, down to its min-content width.
    assert_eq!(
        size(NARROW_ROW, "width:max-content", Body::Text),
        (50.0, 20.0)
    );
    assert_eq!(
        size(NARROW_ROW, "width:fit-content", Body::Text),
        (50.0, 20.0)
    );
    assert_eq!(
        size("display:flex;width:30px", "width:max-content", Body::Text),
        (40.0, 20.0)
    );
}

#[test]
fn a_keyword_width_sizes_a_grid_item_instead_of_stretching_it() {
    assert_eq!(size(GRID, "width:auto", Body::Text), (200.0, 10.0));
    assert_eq!(size(GRID, "width:max-content", Body::Text), (90.0, 10.0));
    assert_eq!(size(GRID, "width:fit-content", Body::Text), (90.0, 10.0));
    assert_eq!(
        size(GRID, "width:fit-content(60px)", Body::Text),
        (60.0, 20.0)
    );
    assert_eq!(size(GRID, "width:min-content", Body::Text), (40.0, 20.0));
    assert_eq!(
        size(NARROW_GRID, "width:fit-content", Body::Text),
        (50.0, 20.0)
    );
    assert_eq!(
        item_rect(GRID, "width:max-content;justify-self:end", Body::Text),
        (110.0, 0.0, 90.0, 10.0)
    );
}

#[test]
fn a_flex_container_item_with_a_keyword_width_is_sized_like_a_block() {
    let flex = "display:flex";
    assert_eq!(
        size(COLUMN, &format!("{flex};width:max-content"), Body::Text),
        (90.0, 10.0)
    );
    assert_eq!(
        size(
            COLUMN,
            &format!("{flex};width:fit-content(60px)"),
            Body::Text
        ),
        (60.0, 20.0)
    );
    assert_eq!(
        size(GRID, &format!("{flex};width:min-content"), Body::Text),
        (40.0, 20.0)
    );
}

#[test]
fn a_fit_content_limit_gives_a_flex_or_grid_item_the_room_it_makes() {
    for container in [COLUMN, ROW, GRID] {
        // A percentage refers to the container's width.
        assert_eq!(
            size(container, "width:fit-content(25%)", Body::Text),
            (50.0, 20.0),
            "{container}"
        );
        // A limit that is zero, or that a calculation makes negative, leaves
        // the min-content width.
        for limit in ["0px", "calc(10px - 50px)", "calc(10% - 50px)"] {
            assert_eq!(
                size(
                    container,
                    &format!("width:fit-content({limit})"),
                    Body::Text
                ),
                (40.0, 20.0),
                "{container} {limit}"
            );
        }
    }
}

#[test]
fn an_inline_box_that_is_a_flex_item_is_sized_by_its_keyword_width() {
    // The item is blockified, so `width` applies to it.
    assert_eq!(
        size(COLUMN, "display:inline;width:max-content", Body::Text),
        (90.0, 10.0)
    );
    assert_eq!(
        size(GRID, "display:inline;width:min-content", Body::Text),
        (40.0, 20.0)
    );
}
