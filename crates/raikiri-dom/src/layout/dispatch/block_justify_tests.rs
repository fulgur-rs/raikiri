//! `justify-self` and `justify-items` place block-level boxes along the inline
//! axis of their block container, where an `auto` width then fits the content
//! (CSS Box Alignment 3 §6.1).

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// The border box `(x, y, width, height)` of a block holding the text `xx`,
/// styled by `item_css`, in a container styled by `container_css`, relative to
/// the container. `before` adds the content that comes before the block.
fn block_rect(
    container_css: &str,
    item_css: &str,
    before: impl FnOnce(&mut crate::Document, usize) -> usize,
) -> (f32, f32, f32, f32) {
    let mut item = 0;
    let fixture = block_fixture(
        &format!("font:10px/10px Ahem;{container_css}"),
        |doc, root| {
            let parent = before(doc, root);
            item = doc.append_element(Some(parent), "div", Some(item_css));
            doc.append_text(item, "xx");
        },
    );
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (container_x, container_y, _, _) = absolute_rect(&doc, root);
    let (x, y, width, height) = absolute_rect(&doc, item);
    (x - container_x, y - container_y, width, height)
}

fn child_rect(container_css: &str, item_css: &str) -> (f32, f32, f32, f32) {
    block_rect(container_css, item_css, |_, root| root)
}

const LTR: &str = "display:block;width:200px";
const RTL: &str = "display:block;width:200px;direction:rtl";

#[test]
fn a_positioned_block_fits_its_content_at_the_alignment() {
    for (container, item, x) in [
        (LTR, "justify-self:start", 0.0),
        (LTR, "justify-self:center", 90.0),
        (LTR, "justify-self:end", 180.0),
        (LTR, "justify-self:flex-end", 180.0),
        (LTR, "justify-self:left", 0.0),
        (LTR, "justify-self:right", 180.0),
        (LTR, "justify-self:baseline", 0.0),
        (RTL, "justify-self:start", 180.0),
        (RTL, "justify-self:end", 0.0),
        (RTL, "justify-self:left", 0.0),
        (RTL, "justify-self:right", 180.0),
        (RTL, "justify-self:baseline", 180.0),
        (RTL, "justify-self:self-start;direction:ltr", 0.0),
    ] {
        let (left, _, width, _) = child_rect(container, &format!("display:block;{item}"));
        assert_eq!((left, width), (x, 20.0), "{container} | {item}");
    }
}

#[test]
fn normal_and_stretch_fill_the_container() {
    for item in [
        "justify-self:normal",
        "justify-self:stretch",
        "justify-self:auto",
    ] {
        let (left, _, width, _) = child_rect(LTR, &format!("display:block;{item}"));
        assert_eq!((left, width), (0.0, 200.0), "{item}");
    }
}

#[test]
fn the_items_of_a_container_take_its_justify_items() {
    let (left, _, width, _) = child_rect(&format!("{LTR};justify-items:center"), "display:block");
    assert_eq!((left, width), (90.0, 20.0));
    let (left, _, width, _) = child_rect(
        &format!("{LTR};justify-items:center"),
        "display:block;justify-self:stretch",
    );
    assert_eq!((left, width), (0.0, 200.0));
}

#[test]
fn auto_margins_take_the_room_before_justify_self() {
    let (left, _, _, _) = child_rect(
        LTR,
        "display:block;width:50px;margin-left:auto;justify-self:start",
    );
    assert_eq!(left, 150.0);
    let (left, _, _, _) = child_rect(
        LTR,
        "display:block;width:50px;margin:0 10px;justify-self:end",
    );
    assert_eq!(left, 140.0);
}

#[test]
fn a_safe_alignment_that_overflows_falls_back_to_the_start() {
    let (left, _, _, _) = child_rect(LTR, "display:block;width:300px;justify-self:safe end");
    assert_eq!(left, 0.0);
    let (left, _, _, _) = child_rect(LTR, "display:block;width:300px;justify-self:unsafe end");
    assert_eq!(left, -100.0);
    let (left, _, _, _) = child_rect(RTL, "display:block;width:300px;justify-self:safe end");
    assert_eq!(left, -100.0);
}

#[test]
fn a_block_in_a_column_is_placed_in_the_column() {
    let multicol = "display:block;width:200px;columns:2;column-gap:0";
    for (container, item, x) in [
        (multicol.to_owned(), "justify-self:start", 0.0),
        (multicol.to_owned(), "justify-self:center", 40.0),
        (multicol.to_owned(), "justify-self:end", 80.0),
        (
            format!("{multicol};direction:rtl"),
            "justify-self:start",
            180.0,
        ),
        (
            format!("{multicol};direction:rtl"),
            "justify-self:end",
            100.0,
        ),
    ] {
        let (left, _, width, _) = child_rect(&container, &format!("display:block;{item}"));
        assert_eq!((left, width), (x, 20.0), "{container} | {item}");
    }
}

#[test]
fn a_block_inside_an_inline_is_placed_in_the_container() {
    for (container, item, x) in [
        (LTR, "justify-self:end", 180.0),
        (LTR, "justify-self:center", 90.0),
        (RTL, "justify-self:start", 180.0),
        (RTL, "justify-self:end", 0.0),
    ] {
        let (left, _, width, _) =
            block_rect(container, &format!("display:block;{item}"), |doc, root| {
                let span = doc.append_element(Some(root), "span", Some("display:inline"));
                doc.append_text(span, "a");
                span
            });
        assert_eq!((left, width), (x, 20.0), "{container} | {item}");
    }
}

#[test]
fn a_vertical_block_is_placed_along_its_inline_axis() {
    let vertical = "display:block;width:100px;height:200px;writing-mode:vertical-rl";
    for (container, item, y) in [
        (vertical.to_owned(), "justify-self:start", 0.0),
        (vertical.to_owned(), "justify-self:center", 90.0),
        (vertical.to_owned(), "justify-self:end", 180.0),
        (
            format!("{vertical};direction:rtl"),
            "justify-self:start",
            180.0,
        ),
        (format!("{vertical};direction:rtl"), "justify-self:end", 0.0),
    ] {
        let (_, top, _, height) = child_rect(&container, &format!("display:block;{item}"));
        assert_eq!((top, height), (y, 20.0), "{container} | {item}");
    }
}
