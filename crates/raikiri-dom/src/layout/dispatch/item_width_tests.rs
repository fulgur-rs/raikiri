//! `width` set to `min-content`, `max-content` or `fit-content` on a flex or
//! grid item takes the size the keyword measures (CSS Sizing 3 §3.1.1), and the
//! item is not stretched; a `min-width` or `max-width` clamps that size, and
//! the lines of the item break at the clamped width. The text of the fixtures
//! is `AAAA BBBB` in Ahem at 10px: 40px wide at its longest word, 90px wide on
//! one line. Every size is the one Chromium gives.

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
fn a_minimum_width_of_zero_does_not_stop_a_keyword_width() {
    for minimum in ["0", "0px", "0%", "calc(0% + 0px)"] {
        assert_eq!(
            size(
                COLUMN,
                &format!("width:max-content;min-width:{minimum}"),
                Body::Text
            ),
            (90.0, 10.0),
            "{minimum}"
        );
    }
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

#[test]
fn an_item_is_in_a_column_flex_container_only_when_the_main_axis_is_vertical() {
    for (container, expected) in [
        ("display:flex;flex-direction:column", true),
        ("display:flex;flex-direction:column-reverse", true),
        ("display:flex;flex-direction:row", false),
        ("display:block", false),
    ] {
        let mut item = 0;
        let fixture = block_fixture(container, |doc, root| {
            item = doc.append_element(Some(root), "div", Some("display:block"));
        });
        let mut doc = fixture.doc;
        layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600())
            .expect("layout");
        let session_cascade = crate::layout::test_support::cascaded(&doc);
        assert_eq!(
            crate::layout::context::LayoutSession::bridged(&mut doc, &session_cascade)
                .in_column_flex_container(item),
            expected,
            "{container}"
        );
    }
}

/// An item holding text, the `min-width` or `max-width` that clamps its keyword
/// width, and the `(width, height)` it takes in a container 200px wide.
const CLAMPED_ITEMS: [(&str, (f32, f32)); 7] = [
    ("width:max-content;max-width:60px", (60.0, 20.0)),
    ("width:max-content;min-width:120px", (120.0, 10.0)),
    ("width:min-content;max-width:30px", (30.0, 20.0)),
    ("width:min-content;min-width:60px", (60.0, 20.0)),
    ("width:fit-content;max-width:50%", (90.0, 10.0)),
    ("width:fit-content;min-width:120px", (120.0, 10.0)),
    ("width:max-content;min-width:50%", (100.0, 10.0)),
];

/// The same for an item that is a flex or grid container itself.
const CLAMPED_CONTAINER_ITEMS: [(&str, (f32, f32)); 5] = [
    (
        "display:grid;width:max-content;max-width:60px",
        (60.0, 20.0),
    ),
    (
        "display:flex;width:max-content;max-width:60px",
        (60.0, 20.0),
    ),
    (
        "display:flex;width:min-content;min-width:60px",
        (60.0, 20.0),
    ),
    (
        "display:grid;width:max-content;min-width:120px",
        (120.0, 10.0),
    ),
    (
        "display:flex;width:fit-content;max-width:60px",
        (60.0, 20.0),
    ),
];

#[test]
fn a_minimum_or_maximum_width_clamps_the_keyword_width_of_an_item() {
    for container in [COLUMN, ROW, GRID] {
        for (style, expected) in CLAMPED_ITEMS {
            assert_eq!(
                size(container, style, Body::Text),
                expected,
                "{container} {style}"
            );
        }
    }
}

#[test]
fn the_lines_of_a_clamped_container_item_break_at_the_width_it_keeps() {
    for container in [COLUMN, ROW, GRID] {
        for (style, expected) in CLAMPED_CONTAINER_ITEMS {
            assert_eq!(
                size(container, style, Body::Text),
                expected,
                "{container} {style}"
            );
        }
    }
}

#[test]
fn a_clamped_keyword_width_adds_the_padding_and_border_of_the_item() {
    let edges =
        "box-sizing:content-box;padding:0 5px;border:2px solid;width:max-content;max-width:60px";
    for container in [COLUMN, ROW, GRID] {
        assert_eq!(size(container, edges, Body::Text), (74.0, 24.0));
        assert_eq!(
            size(container, &format!("display:grid;{edges}"), Body::Text),
            (74.0, 24.0)
        );
    }
}

#[test]
fn a_clamped_container_item_sizes_a_min_content_grid_around_it() {
    // The item asks for its max-content width, which its `max-width` cuts to
    // 50px; that is the min-content width of the grid around it too.
    assert_eq!(
        item_rect(
            "display:grid;width:min-content",
            "display:grid;width:max-content;max-width:50px",
            Body::Text
        ),
        (0.0, 0.0, 50.0, 20.0)
    );
}

#[test]
fn a_keyword_minimum_or_maximum_width_clamps_the_keyword_width_of_a_container_item() {
    assert_eq!(
        size(
            ROW,
            "display:grid;width:max-content;max-width:min-content",
            Body::Text
        ),
        (40.0, 20.0)
    );
    assert_eq!(
        size(
            ROW,
            "display:flex;width:max-content;max-width:min-content",
            Body::Text
        ),
        (40.0, 20.0)
    );
    assert_eq!(
        size(
            ROW,
            "display:grid;width:min-content;min-width:max-content",
            Body::Text
        ),
        (90.0, 10.0)
    );
}

#[test]
fn a_content_flex_basis_takes_the_content_width_of_a_clamped_container_item() {
    // `flex-basis: content` sets aside the keyword width; only the limits
    // clamp the width of the content.
    for display in ["display:grid", "display:flex"] {
        assert_eq!(
            size(
                ROW,
                &format!("{display};flex:0 0 content;width:max-content;max-width:60px"),
                Body::Text
            ),
            (60.0, 20.0),
            "{display}"
        );
        assert_eq!(
            size(
                ROW,
                &format!("{display};flex:0 0 content;width:min-content;min-width:60px"),
                Body::Text
            ),
            (90.0, 10.0),
            "{display}"
        );
    }
}
