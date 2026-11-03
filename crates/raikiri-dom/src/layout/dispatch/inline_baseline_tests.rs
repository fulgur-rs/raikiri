//! The work baseline alignment along the inline axis does for a container
//! grows with the number of its items, not with the square of it.
//!
//! A pass over the items may run more than once for a layout, so the tests
//! compare the work for a number of items with that for four times as many:
//! it grows four times when it grows with the number of items, and sixteen
//! times when it grows with the square of it.

use crate::Document;
use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{
    INLINE_BASELINE_MEMBER_VISITS, INLINE_BASELINE_SHIFT_PROBES, absolute_rect, page_box_800x600,
    with_ahem,
};

/// Lay out `container_css` holding the elements `build` adds, and return the
/// document and the container.
fn laid_out(container_css: &str, build: impl FnOnce(&mut Document, usize)) -> (Document, usize) {
    let fixture = block_fixture(container_css, build);
    let root = fixture.root;
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    (doc, root)
}

/// The most that the work for four times as many items may be, as a multiple
/// of the work for the number of items: between four and sixteen.
const MOST_GROWTH: usize = 6;

/// Lay out a grid whose `count` items each sit alone in a column of their own,
/// so that every item is a baseline-sharing group of its own, and check where
/// the items are. Returns how many times the pass looked at an item.
fn grid_of_groups(count: usize) -> usize {
    let mut items = Vec::new();
    INLINE_BASELINE_MEMBER_VISITS.with(|visits| visits.set(0));
    let (doc, root) = laid_out(
        &format!("display:grid;justify-items:baseline;grid-template-columns:repeat({count},1px)"),
        |doc, root| {
            for index in 0..count {
                let style = format!(
                    "display:block;grid-column:{};grid-row:1;margin-left:{}px",
                    index + 1,
                    index % 3
                );
                items.push(doc.append_element(Some(root), "div", Some(style.as_str())));
            }
        },
    );
    let (left, ..) = absolute_rect(&doc, root);
    for (index, &item) in items.iter().enumerate() {
        let (x, ..) = absolute_rect(&doc, item);
        assert_eq!(x, left + index as f32 + (index % 3) as f32, "item {index}");
    }
    INLINE_BASELINE_MEMBER_VISITS.with(|visits| visits.get())
}

#[test]
fn items_that_are_all_in_groups_of_their_own_are_aligned_without_comparing_each_to_the_others() {
    let few = grid_of_groups(128);
    let many = grid_of_groups(512);
    // Every item is looked at to find the depth of its group.
    assert!(few >= 128, "{few} visits for 128 items");
    assert!(
        many <= few * MOST_GROWTH,
        "{few} visits for 128 items, {many} for 512"
    );
}

/// Lay out a column flex container that aligns its `count` items by baseline,
/// where the first has a margin of 7px that the others have none of, so that
/// all of the others move 7px. Each item holds an absolutely positioned box
/// with no inset; `containing` makes the container its containing block when
/// it is `position:relative`, and leaves that to the page otherwise. Checks
/// that every box follows its item. Returns how many steps were taken to find
/// the item that holds a box, and to look for it among the items that moved.
fn flex_of_moved_items(count: usize, containing: &str) -> usize {
    let (mut items, mut boxes) = (Vec::new(), Vec::new());
    INLINE_BASELINE_SHIFT_PROBES.with(|probes| probes.set(0));
    let (doc, root) = laid_out(
        &format!("display:flex;flex-direction:column;align-items:baseline;{containing}"),
        |doc, root| {
            for index in 0..count {
                let style = if index == 0 {
                    "display:block;margin-left:7px"
                } else {
                    "display:block"
                };
                let item = doc.append_element(Some(root), "div", Some(style));
                boxes.push(doc.append_element(
                    Some(item),
                    "div",
                    Some("display:block;position:absolute;width:3px;height:3px"),
                ));
                items.push(item);
            }
        },
    );
    let (left, ..) = absolute_rect(&doc, root);
    for (index, (&item, &abs)) in items.iter().zip(&boxes).enumerate() {
        let (item_x, ..) = absolute_rect(&doc, item);
        let (box_x, ..) = absolute_rect(&doc, abs);
        // Every item sits 7px from the container, and the box follows it.
        assert_eq!(item_x, left + 7.0, "item {index}, {containing:?}");
        assert_eq!(box_x, item_x, "box {index}, {containing:?}");
    }
    INLINE_BASELINE_SHIFT_PROBES.with(|probes| probes.get())
}

#[test]
fn the_boxes_that_follow_the_moved_items_are_not_searched_for_among_all_of_them() {
    for containing in ["", "position:relative"] {
        let few = flex_of_moved_items(128, containing);
        let many = flex_of_moved_items(512, containing);
        // Each box takes a step or two up to its item.
        assert!(few >= 128, "{few} steps for 128 boxes, {containing:?}");
        assert!(
            many <= few * MOST_GROWTH,
            "{few} steps for 128 boxes, {many} for 512, {containing:?}"
        );
    }
}
