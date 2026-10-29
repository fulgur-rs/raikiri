//! A horizontal child of a vertical container is an orthogonal flow: its inline
//! size is its physical width, which a `fit-content` or `min-content` keyword
//! sizes from the room the container leaves it (CSS Writing Modes 4 §7.3.1,
//! CSS Sizing 3 §3.1.1). The rects are those of Chromium for the twelve words
//! of [`WORDS`] in a 100x200 container.

use super::{WORDS, layout, layout_after_text};
use crate::Document;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, ifc_ahem_fonts, page_box_800x600, with_ahem};

type Rect = (f32, f32, f32, f32);

/// A `width` and the rects of the child it styles: in a container of blocks
/// only, and in one that holds the text `xx` before the child, where the
/// paragraph root of the container lays its children out.
type Case = (&'static str, Rect, Rect);

const VERTICAL_RL: [Case; 12] = [
    (
        "width:fit-content",
        (8.0, 8.0, 100.0, 40.0),
        (-2.0, 8.0, 100.0, 40.0),
    ),
    (
        "width:min-content",
        (88.0, 8.0, 20.0, 120.0),
        (78.0, 8.0, 20.0, 120.0),
    ),
    (
        "width:max-content",
        (-242.0, 8.0, 350.0, 10.0),
        (-252.0, 8.0, 350.0, 10.0),
    ),
    (
        "width:fit-content;margin:0 10px",
        (18.0, 8.0, 80.0, 40.0),
        (8.0, 8.0, 80.0, 40.0),
    ),
    (
        "width:min-content;margin:0 10px",
        (78.0, 8.0, 20.0, 120.0),
        (68.0, 8.0, 20.0, 120.0),
    ),
    (
        "width:min-content;min-width:60px",
        (48.0, 8.0, 60.0, 60.0),
        (38.0, 8.0, 60.0, 60.0),
    ),
    (
        "width:fit-content;max-width:60px",
        (48.0, 8.0, 60.0, 60.0),
        (38.0, 8.0, 60.0, 60.0),
    ),
    (
        "width:min-content;max-width:10px",
        (98.0, 8.0, 10.0, 120.0),
        (88.0, 8.0, 10.0, 120.0),
    ),
    (
        "width:fit-content;min-width:150px",
        (-42.0, 8.0, 150.0, 30.0),
        (-52.0, 8.0, 150.0, 30.0),
    ),
    (
        "width:fit-content;padding:0 5px;border:2px solid",
        (8.0, 8.0, 100.0, 44.0),
        (-2.0, 8.0, 100.0, 44.0),
    ),
    (
        "width:min-content;padding:0 5px;border:2px solid",
        (74.0, 8.0, 34.0, 124.0),
        (64.0, 8.0, 34.0, 124.0),
    ),
    (
        "width:fit-content;box-sizing:content-box;padding:0 5px;margin:0 5px",
        (13.0, 8.0, 90.0, 40.0),
        (3.0, 8.0, 90.0, 40.0),
    ),
];

const VERTICAL_LR: [Case; 12] = [
    (
        "width:fit-content",
        (8.0, 8.0, 100.0, 40.0),
        (18.0, 8.0, 100.0, 40.0),
    ),
    (
        "width:min-content",
        (8.0, 8.0, 20.0, 120.0),
        (18.0, 8.0, 20.0, 120.0),
    ),
    (
        "width:max-content",
        (8.0, 8.0, 350.0, 10.0),
        (18.0, 8.0, 350.0, 10.0),
    ),
    (
        "width:fit-content;margin:0 10px",
        (18.0, 8.0, 80.0, 40.0),
        (28.0, 8.0, 80.0, 40.0),
    ),
    (
        "width:min-content;margin:0 10px",
        (18.0, 8.0, 20.0, 120.0),
        (28.0, 8.0, 20.0, 120.0),
    ),
    (
        "width:min-content;min-width:60px",
        (8.0, 8.0, 60.0, 60.0),
        (18.0, 8.0, 60.0, 60.0),
    ),
    (
        "width:fit-content;max-width:60px",
        (8.0, 8.0, 60.0, 60.0),
        (18.0, 8.0, 60.0, 60.0),
    ),
    (
        "width:min-content;max-width:10px",
        (8.0, 8.0, 10.0, 120.0),
        (18.0, 8.0, 10.0, 120.0),
    ),
    (
        "width:fit-content;min-width:150px",
        (8.0, 8.0, 150.0, 30.0),
        (18.0, 8.0, 150.0, 30.0),
    ),
    (
        "width:fit-content;padding:0 5px;border:2px solid",
        (8.0, 8.0, 100.0, 44.0),
        (18.0, 8.0, 100.0, 44.0),
    ),
    (
        "width:min-content;padding:0 5px;border:2px solid",
        (8.0, 8.0, 34.0, 124.0),
        (18.0, 8.0, 34.0, 124.0),
    ),
    (
        "width:fit-content;box-sizing:content-box;padding:0 5px;margin:0 5px",
        (13.0, 8.0, 90.0, 40.0),
        (23.0, 8.0, 90.0, 40.0),
    ),
];

/// Percentages of the padding refer to the 200px inline size of the container,
/// its physical height, whatever the width the keyword measures.
const PERCENT_PADDING_VERTICAL_RL: [Case; 4] = [
    (
        "width:fit-content;padding:10%",
        (8.0, 8.0, 100.0, 100.0),
        (-2.0, 8.0, 100.0, 100.0),
    ),
    (
        "width:min-content;padding:10%",
        (48.0, 8.0, 60.0, 160.0),
        (38.0, 8.0, 60.0, 160.0),
    ),
    (
        "width:min-content;padding:5%",
        (68.0, 8.0, 40.0, 140.0),
        (58.0, 8.0, 40.0, 140.0),
    ),
    (
        "width:fit-content;padding:0 10%;margin:0 5%",
        (18.0, 8.0, 80.0, 120.0),
        (8.0, 8.0, 80.0, 120.0),
    ),
];

/// The rects of a horizontal flex container of six 30px boxes, 180px wide on
/// one line and 30px wide on six. A root that holds no text lays its children
/// out on transposed axes, not as a paragraph.
const FLEX_BOXES_VERTICAL_RL: [(&str, Rect); 14] = [
    ("width:fit-content", (8.0, 8.0, 100.0, 20.0)),
    ("width:min-content", (78.0, 8.0, 30.0, 60.0)),
    ("width:max-content", (-72.0, 8.0, 180.0, 10.0)),
    ("width:auto", (8.0, 8.0, 100.0, 20.0)),
    ("width:fit-content;margin:0 10px", (18.0, 8.0, 80.0, 30.0)),
    ("width:min-content;margin:0 10px", (68.0, 8.0, 30.0, 60.0)),
    ("width:min-content;min-width:60px", (48.0, 8.0, 60.0, 30.0)),
    ("width:fit-content;max-width:60px", (48.0, 8.0, 60.0, 30.0)),
    ("width:min-content;max-width:10px", (98.0, 8.0, 10.0, 60.0)),
    (
        "width:fit-content;min-width:150px",
        (-42.0, 8.0, 150.0, 20.0),
    ),
    (
        "width:fit-content;padding:0 5px;border:2px solid",
        (8.0, 8.0, 100.0, 34.0),
    ),
    (
        "width:min-content;padding:0 5px;border:2px solid",
        (64.0, 8.0, 44.0, 64.0),
    ),
    ("width:fit-content;padding:10%", (8.0, 8.0, 100.0, 70.0)),
    ("width:min-content;padding:10%", (38.0, 8.0, 70.0, 100.0)),
];

const FLEX_BOXES_VERTICAL_LR: [(&str, Rect); 14] = [
    ("width:fit-content", (8.0, 8.0, 100.0, 20.0)),
    ("width:min-content", (8.0, 8.0, 30.0, 60.0)),
    ("width:max-content", (8.0, 8.0, 180.0, 10.0)),
    ("width:auto", (8.0, 8.0, 100.0, 20.0)),
    ("width:fit-content;margin:0 10px", (18.0, 8.0, 80.0, 30.0)),
    ("width:min-content;margin:0 10px", (18.0, 8.0, 30.0, 60.0)),
    ("width:min-content;min-width:60px", (8.0, 8.0, 60.0, 30.0)),
    ("width:fit-content;max-width:60px", (8.0, 8.0, 60.0, 30.0)),
    ("width:min-content;max-width:10px", (8.0, 8.0, 10.0, 60.0)),
    ("width:fit-content;min-width:150px", (8.0, 8.0, 150.0, 20.0)),
    (
        "width:fit-content;padding:0 5px;border:2px solid",
        (8.0, 8.0, 100.0, 34.0),
    ),
    (
        "width:min-content;padding:0 5px;border:2px solid",
        (8.0, 8.0, 44.0, 64.0),
    ),
    ("width:fit-content;padding:10%", (8.0, 8.0, 100.0, 70.0)),
    ("width:min-content;padding:10%", (8.0, 8.0, 70.0, 100.0)),
];

fn check(container: &str, cases: &[Case]) {
    let css = format!("writing-mode:{container};width:100px;height:200px");
    for (width, blocks_only, with_text) in cases {
        let child = format!("writing-mode:horizontal-tb;box-sizing:border-box;{width}|{WORDS}");
        assert_eq!(
            layout(&css, &[&child])[1],
            *blocks_only,
            "{container} {width}"
        );
        assert_eq!(
            layout_after_text(&css, "xx", &[&child])[1],
            *with_text,
            "{container} {width} after text"
        );
    }
}

fn check_flex_boxes(container: &str, cases: &[(&str, Rect)]) {
    let css = format!("writing-mode:{container};width:100px;height:200px");
    for (width, expected) in cases {
        let child = format!(
            "writing-mode:horizontal-tb;box-sizing:border-box;display:flex;flex-wrap:wrap;\
             align-content:flex-start;{width}"
        );
        assert_eq!(
            super::horizontal_child_rect(&css, &child, &["width:30px;height:10px"; 6]),
            *expected,
            "{container} {width}"
        );
    }
}

#[test]
fn horizontal_child_of_a_vertical_rl_container_takes_the_width_its_keyword_measures() {
    check("vertical-rl", &VERTICAL_RL);
}

#[test]
fn horizontal_child_of_a_vertical_lr_container_takes_the_width_its_keyword_measures() {
    check("vertical-lr", &VERTICAL_LR);
}

#[test]
fn horizontal_child_keyword_width_resolves_percentage_padding_against_the_container() {
    check("vertical-rl", &PERCENT_PADDING_VERTICAL_RL);
}

#[test]
fn horizontal_flex_child_of_a_container_without_text_takes_the_width_its_keyword_measures() {
    check_flex_boxes("vertical-rl", &FLEX_BOXES_VERTICAL_RL);
    check_flex_boxes("vertical-lr", &FLEX_BOXES_VERTICAL_LR);
}

#[test]
fn container_without_text_or_width_is_as_wide_as_the_keyword_width_of_its_child() {
    for (width, expected) in [
        ("width:fit-content", (8.0, 8.0, 180.0, 10.0)),
        ("width:min-content", (8.0, 8.0, 30.0, 60.0)),
        ("width:fit-content;margin:0 10px", (18.0, 8.0, 180.0, 10.0)),
    ] {
        let child = format!(
            "writing-mode:horizontal-tb;display:flex;flex-wrap:wrap;align-content:flex-start;{width}"
        );
        assert_eq!(
            super::horizontal_child_rect(
                "writing-mode:vertical-rl;height:200px",
                &child,
                &["width:30px;height:10px"; 6]
            ),
            expected,
            "{width}"
        );
    }
}

#[test]
fn vertical_container_without_a_width_is_as_wide_as_the_keyword_width_of_its_child() {
    // The room is the 800px page area, wider than the 350px of the words.
    // `fit-content` takes them whole, `min-content` the longest word.
    for (width, container, child) in [
        (
            "fit-content",
            (8.0, 8.0, 350.0, 200.0),
            (8.0, 8.0, 350.0, 10.0),
        ),
        (
            "min-content",
            (8.0, 8.0, 20.0, 200.0),
            (8.0, 8.0, 20.0, 120.0),
        ),
    ] {
        let rects = layout(
            "writing-mode:vertical-rl;height:200px",
            &[&format!("writing-mode:horizontal-tb;width:{width}|{WORDS}")],
        );
        assert_eq!(rects[0], container, "{width}");
        assert_eq!(rects[1], child, "{width}");
    }
}

#[test]
fn horizontal_flex_and_grid_children_take_the_width_their_keyword_measures() {
    // Two items of "aa bb cc" and "dd ee ff", 80px wide at most and 20px at
    // the narrowest. The flex container puts them side by side, the grid
    // container one above the other.
    let css = "writing-mode:vertical-rl;width:100px;height:200px";
    for (display, width, expected) in [
        ("flex", "fit-content", (8.0, 8.0, 100.0, 20.0)),
        ("flex", "min-content", (68.0, 8.0, 40.0, 30.0)),
        ("grid", "fit-content", (28.0, 8.0, 80.0, 20.0)),
        ("grid", "min-content", (88.0, 8.0, 20.0, 60.0)),
    ] {
        let child = format!("writing-mode:horizontal-tb;display:{display};width:{width}");
        assert_eq!(
            super::horizontal_child_rect(css, &child, &["|aa bb cc", "|dd ee ff"]),
            expected,
            "{display} {width}"
        );
    }
}

/// The rects of the vertical root `root_css`, which shrinks to fit inside the
/// 100px wide box `parent_css`, and of its horizontal child `width`.
fn shrink_to_fit_root(parent_css: &str, root_css: &str, width: &str) -> (Rect, Rect) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:8px"));
    let parent = doc.append_element(
        Some(body),
        "div",
        Some(
            format!("width:100px;font-family:Ahem;font-size:10px;line-height:10px;{parent_css}")
                .as_str(),
        ),
    );
    let root = doc.append_element(
        Some(parent),
        "div",
        Some(format!("writing-mode:vertical-rl;height:200px;{root_css}").as_str()),
    );
    let child = doc.append_element(
        Some(root),
        "div",
        Some(format!("display:block;writing-mode:horizontal-tb;{width}").as_str()),
    );
    doc.append_text(child, WORDS);
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    doc.set_font_collection_with_limits(ifc_ahem_fonts(), shodo::limits::Limits::default());
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    (absolute_rect(&doc, root), absolute_rect(&doc, child))
}

#[test]
fn vertical_root_that_shrinks_to_fit_takes_the_keyword_width_of_its_child() {
    // The root does not fit the 100px its parent leaves it: a child measured
    // for the root takes the page area as its room, so `auto` and
    // `fit-content` stay 350px wide, while `min-content` is as wide as the
    // longest word.
    for (parent, root) in [
        ("", "float:left"),
        ("", "display:inline-block"),
        ("display:flex", ""),
    ] {
        for (width, root_rect, child_rect) in [
            (
                "width:auto",
                (8.0, 8.0, 350.0, 200.0),
                (8.0, 8.0, 350.0, 10.0),
            ),
            (
                "width:fit-content",
                (8.0, 8.0, 350.0, 200.0),
                (8.0, 8.0, 350.0, 10.0),
            ),
            (
                "width:min-content",
                (8.0, 8.0, 20.0, 200.0),
                (8.0, 8.0, 20.0, 120.0),
            ),
        ] {
            assert_eq!(
                shrink_to_fit_root(parent, root, width),
                (root_rect, child_rect),
                "{parent} {root} {width}"
            );
        }
    }
}
