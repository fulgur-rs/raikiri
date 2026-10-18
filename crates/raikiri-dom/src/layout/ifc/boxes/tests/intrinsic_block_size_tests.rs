//! A block child of a paragraph root whose size along the root's inline axis
//! is `min-content`, `max-content` or `fit-content` takes its content size,
//! not the room the root leaves it (CSS Sizing 3 §5.1).

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// The border box size `(width, height)` of a block child laid out after
/// `lead`, in a root styled by `root_css`. The child's text is `AAAA BBBB`.
fn child_size(root_css: &str, lead: &str, child_css: &str) -> (f32, f32) {
    let mut child = 0;
    let fixture = block_fixture(&format!("line-height:10px;{root_css}"), |doc, root| {
        if !lead.is_empty() {
            doc.append_text(root, lead);
        }
        child = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some(&format!("display:block;{child_css}")),
        );
        doc.append_text(child, "AAAA BBBB");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (_, _, width, height) = absolute_rect(&doc, child);
    (width, height)
}

#[test]
fn a_block_in_a_paragraph_shrinks_to_its_min_content_width() {
    // Each of the two words is 40px wide, so the box wraps at the space.
    assert_eq!(
        child_size("width: 100px", "x", "width: min-content"),
        (40.0, 20.0)
    );
    // Without a keyword it fills the room, the line staying whole.
    assert_eq!(child_size("width: 100px", "x", ""), (100.0, 10.0));
}

#[test]
fn a_block_with_its_own_formatting_context_shrinks_to_its_min_content_width() {
    assert_eq!(
        child_size("width: 100px", "x", "overflow: hidden; width: min-content"),
        (40.0, 20.0)
    );
}

#[test]
fn a_min_content_width_stays_within_the_max_width() {
    assert_eq!(
        child_size("width: 100px", "x", "width: min-content; max-width: 30px"),
        (30.0, 20.0)
    );
}

#[test]
fn a_block_of_a_vertical_root_takes_its_min_content_inline_size() {
    let root = "writing-mode: vertical-lr; width: auto; height: 100px";
    // The inline axis is the physical height: the words stack in two lines.
    assert_eq!(
        child_size(root, "x", "inline-size: min-content"),
        (20.0, 40.0)
    );
    assert_eq!(child_size(root, "x", ""), (10.0, 100.0));
}

#[test]
fn a_min_content_inline_size_may_exceed_the_vertical_root() {
    // The unbreakable text is 90px long in the 50px root.
    let root = "writing-mode: vertical-lr; width: auto; height: 50px";
    assert_eq!(
        child_size(root, "x", "inline-size: min-content; white-space: pre"),
        (10.0, 90.0)
    );
}

#[test]
fn a_block_in_a_paragraph_takes_its_max_content_width() {
    // The two words stay on one line, 90px wide, though the root is 60px.
    assert_eq!(
        child_size("width: 60px", "x", "width: max-content"),
        (90.0, 10.0)
    );
    assert_eq!(child_size("width: 60px", "x", ""), (60.0, 20.0));
    // The same without a paragraph around it: a block in a block.
    assert_eq!(
        child_size("width: 60px", "", "width: max-content"),
        (90.0, 10.0)
    );
    assert_eq!(child_size("width: 60px", "", ""), (60.0, 20.0));
}

#[test]
fn a_block_with_its_own_formatting_context_takes_its_max_content_width() {
    assert_eq!(
        child_size("width: 60px", "x", "overflow: hidden; width: max-content"),
        (90.0, 10.0)
    );
}

#[test]
fn a_fit_content_width_is_the_room_between_min_and_max_content() {
    // Room for the whole line: the max-content width.
    assert_eq!(
        child_size("width: 200px", "x", "width: fit-content"),
        (90.0, 10.0)
    );
    // Less room than that, more than the longest word: the room.
    assert_eq!(
        child_size("width: 60px", "x", "width: fit-content"),
        (60.0, 20.0)
    );
    // Less room than the longest word: the min-content width.
    assert_eq!(
        child_size("width: 30px", "x", "width: fit-content"),
        (40.0, 20.0)
    );
    assert_eq!(
        child_size("width: 60px", "", "width: fit-content"),
        (60.0, 20.0)
    );
}

#[test]
fn a_fit_content_width_leaves_room_for_the_margins() {
    // 60px less the 10px margins on both sides.
    assert_eq!(
        child_size("width: 60px", "x", "width: fit-content; margin: 0 10px"),
        (40.0, 20.0)
    );
}

#[test]
fn a_block_beside_a_float_fits_its_content_to_the_room_the_float_leaves() {
    // The float takes 40px of the 100px root, leaving 60px.
    let mut child = 0;
    let fixture = block_fixture("line-height:10px;width: 100px", |doc, root| {
        let float = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some("display:block;float:left;width:40px;height:10px"),
        );
        doc.append_text(float, "");
        child = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some("display:block;overflow:hidden;width:fit-content"),
        );
        doc.append_text(child, "AAAA BBBB");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (x, _, width, _) = absolute_rect(&doc, child);
    assert_eq!((x, width), (40.0, 60.0));
}

#[test]
fn a_max_content_width_stays_within_the_max_width() {
    assert_eq!(
        child_size("width: 100px", "x", "width: max-content; max-width: 30px"),
        (30.0, 20.0)
    );
}

#[test]
fn a_block_of_a_vertical_root_takes_its_max_content_inline_size() {
    let root = "writing-mode: vertical-lr; width: auto; height: 50px";
    // The inline axis is the physical height: the words stay in one line.
    assert_eq!(
        child_size(root, "x", "inline-size: max-content"),
        (10.0, 90.0)
    );
    assert_eq!(
        child_size(root, "x", "inline-size: fit-content"),
        (20.0, 50.0)
    );
    assert_eq!(child_size(root, "x", ""), (20.0, 50.0));
}

#[test]
fn a_flex_container_fits_its_content_to_the_room_the_root_leaves() {
    // The anonymous flex item holds `AAAA BBBB`: 90px wide, 40px at its
    // longest word. Taffy reads a flex container's `fit-content` as
    // `max-content`, so the limit has to be applied on top of it.
    for display in ["flex", "inline-flex"] {
        for lead in ["", "x"] {
            for (root, expected) in [
                ("width: 200px", (90.0, 10.0)),
                ("width: 60px", (60.0, 20.0)),
                ("width: 30px", (40.0, 20.0)),
            ] {
                assert_eq!(
                    child_size(
                        root,
                        lead,
                        &format!("display: {display}; width: fit-content")
                    ),
                    expected,
                    "{display}, {root}, lead {lead:?}"
                );
            }
        }
        assert_eq!(
            child_size(
                "width: 60px",
                "",
                &format!("display: {display}; width: max-content")
            ),
            (90.0, 10.0)
        );
    }
}

#[test]
fn a_floated_or_positioned_flex_container_fits_its_content_to_the_room() {
    assert_eq!(
        child_size(
            "width: 60px",
            "",
            "float: left; display: flex; width: fit-content"
        ),
        (60.0, 20.0)
    );
    let css = "position: absolute; display: flex; width: fit-content";
    for (root, expected) in [
        ("position: relative; width: 200px", (90.0, 10.0)),
        ("position: relative; width: 60px", (60.0, 20.0)),
        ("position: relative; width: 30px", (40.0, 20.0)),
    ] {
        assert_eq!(child_size(root, "", css), expected, "{root}");
    }
}

#[test]
fn a_flex_container_with_a_fit_content_width_obeys_the_min_and_max_width() {
    assert_eq!(
        child_size(
            "width: 60px",
            "",
            "display: flex; width: fit-content; max-width: 50px"
        ),
        (50.0, 20.0)
    );
    assert_eq!(
        child_size(
            "width: 60px",
            "",
            "display: flex; width: fit-content; min-width: 80px"
        ),
        (80.0, 20.0)
    );
    assert_eq!(
        child_size(
            "width: 60px",
            "",
            "display: flex; width: fit-content; margin: 0 10px"
        ),
        (40.0, 20.0)
    );
}

#[test]
fn a_grid_container_and_a_table_fit_their_content_to_the_room_the_root_leaves() {
    for display in ["grid", "table"] {
        let css = format!("display: {display}; width: fit-content");
        assert_eq!(child_size("width: 200px", "", &css), (90.0, 10.0));
        assert_eq!(child_size("width: 60px", "", &css), (60.0, 20.0));
        assert_eq!(child_size("width: 60px", "x", &css), (60.0, 20.0));
        assert_eq!(child_size("width: 30px", "", &css), (40.0, 20.0));
        let css = format!("display: {display}; width: max-content");
        assert_eq!(child_size("width: 60px", "", &css), (90.0, 10.0));
    }
}

#[test]
fn a_float_and_an_inline_block_take_a_fit_content_width() {
    for css in ["float: left", "display: inline-block"] {
        let css = format!("{css}; width: fit-content");
        assert_eq!(child_size("width: 60px", "", &css), (60.0, 20.0));
        assert_eq!(child_size("width: 30px", "", &css), (40.0, 20.0));
    }
}

#[test]
fn an_absolutely_positioned_box_takes_a_fit_content_width_from_its_containing_block() {
    let root = "position: relative; width: 60px";
    let css = "position: absolute; width: fit-content";
    assert_eq!(child_size(root, "", css), (60.0, 20.0));
    assert_eq!(
        child_size("position: relative; width: 30px", "", css),
        (40.0, 20.0)
    );
    assert_eq!(
        child_size("position: relative; width: 200px", "", css),
        (90.0, 10.0)
    );
    // The insets take their share of the room.
    assert_eq!(
        child_size(
            root,
            "",
            "position: absolute; left: 10px; right: 10px; width: fit-content"
        ),
        (40.0, 20.0)
    );
    assert_eq!(
        child_size(root, "", "position: absolute; width: max-content"),
        (90.0, 10.0)
    );
}

#[test]
fn a_content_keyword_width_adds_the_padding_and_obeys_the_min_and_max_width() {
    assert_eq!(
        child_size("width: 60px", "", "width: max-content; padding: 0 5px"),
        (100.0, 10.0)
    );
    assert_eq!(
        child_size("width: 60px", "", "width: max-content; min-width: 100px"),
        (100.0, 10.0)
    );
    assert_eq!(
        child_size("width: 60px", "", "width: max-content; max-width: 50%"),
        (30.0, 20.0)
    );
}
