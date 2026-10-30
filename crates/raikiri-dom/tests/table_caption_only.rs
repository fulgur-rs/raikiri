//! A table with a caption and no rows or cells is as wide as the caption
//! needs, not as wide as its padding and border: the caption is at least as
//! wide as its min-content size, and the table around it as wide as the
//! caption (CSS Tables 3 §3, CAPMIN).
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px: each is the caption's
//! left, top, width and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the element `id` in `body`, on the first
/// page of 800px by 600px.
fn border_box(body: &str, id: &str) -> [f32; 4] {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .map(|fragment| {
            let rect = fragment.rect();
            [rect.x, rect.y, rect.width, rect.height]
        })
        .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a table with a caption `c` and no rows (the style of
/// the table and of the caption, and the caption's text), the markup around it,
/// and the caption box Chromium gives.
struct Case {
    label: &'static str,
    table_style: &'static str,
    caption_style: &'static str,
    text: &'static str,
    /// The markup before and after the table.
    around: (&'static str, &'static str),
    caption: [f32; 4],
}

const fn case(
    label: &'static str,
    table_style: &'static str,
    caption_style: &'static str,
    text: &'static str,
    caption: [f32; 4],
) -> Case {
    Case {
        label,
        table_style,
        caption_style,
        text,
        around: ("", ""),
        caption,
    }
}

fn check(cases: &[Case]) {
    for case in cases {
        let body = format!(
            "{}<div style=\"display:table;{}\"><div id=c style=\"display:table-caption;{}\">{}</div></div>{}",
            case.around.0, case.table_style, case.caption_style, case.text, case.around.1
        );
        let found = border_box(&body, "c");
        assert!(
            found
                .iter()
                .zip(case.caption)
                .all(|(got, want)| (got - want).abs() <= TOLERANCE),
            "{}: the caption is {found:?}, expected {:?} (left, top, width, height)",
            case.label,
            case.caption
        );
    }
}

#[test]
fn a_table_with_only_a_caption_is_as_wide_as_the_caption_needs() {
    check(&[
        case("one letter", "", "", "1", [8.0, 8.0, 10.0, 10.0]),
        // The caption wraps at the spaces: the widest word sets the width.
        case("several words", "", "", "aa bbb cc", [8.0, 8.0, 30.0, 30.0]),
        case("one long word", "", "", "aaaaaaa", [8.0, 8.0, 70.0, 10.0]),
        case("no text", "", "", "", [8.0, 8.0, 0.0, 0.0]),
        case(
            "below the rows",
            "",
            "caption-side:bottom",
            "aa bbb cc",
            [8.0, 8.0, 30.0, 30.0],
        ),
        case(
            "padding and border of the caption",
            "",
            "padding:3px;border:2px solid",
            "aa bbb cc",
            [8.0, 8.0, 40.0, 40.0],
        ),
        case(
            "margins of the caption",
            "",
            "margin:0 7px",
            "aaaa",
            [15.0, 8.0, 40.0, 10.0],
        ),
        case(
            "a width of the caption",
            "",
            "width:100px",
            "aaaa",
            [8.0, 8.0, 100.0, 10.0],
        ),
        // The padding and border of the table are inside its width.
        case(
            "padding and border of the table",
            "padding:5px;border:4px solid",
            "",
            "aa bbb cc",
            [8.0, 8.0, 30.0, 30.0],
        ),
    ]);
}

#[test]
fn a_table_with_only_a_caption_keeps_the_caption_min_content_width_whatever_it_states() {
    check(&[
        case(
            "a width",
            "width:200px",
            "",
            "aa bb",
            [8.0, 8.0, 200.0, 10.0],
        ),
        case(
            "a width below the caption",
            "width:20px",
            "",
            "aa bbb cc",
            [8.0, 8.0, 30.0, 30.0],
        ),
        case(
            "a max-width below the caption",
            "max-width:20px",
            "",
            "aa bbb cc",
            [8.0, 8.0, 30.0, 30.0],
        ),
    ]);
}

#[test]
fn a_table_with_only_a_caption_shrinks_to_the_caption_in_every_context() {
    let around = |before, after| (before, after);
    let in_box = |label, before| Case {
        label,
        table_style: "",
        caption_style: "",
        text: "aa bbb cc",
        around: around(before, "</div>"),
        caption: [8.0, 8.0, 30.0, 30.0],
    };
    check(&[
        case(
            "a float",
            "float:left",
            "",
            "aa bbb cc",
            [8.0, 8.0, 30.0, 30.0],
        ),
        case(
            "an inline-table",
            "display:inline-table",
            "",
            "aa bbb cc",
            [8.0, 8.0, 30.0, 30.0],
        ),
        case(
            "absolutely positioned",
            "position:absolute",
            "",
            "aa bbb cc",
            [8.0, 8.0, 30.0, 30.0],
        ),
        case(
            "a vertical table",
            "writing-mode:vertical-rl",
            "",
            "aa bbb cc",
            [8.0, 8.0, 30.0, 30.0],
        ),
        // The caption is as wide as it needs, not as the room around the table.
        in_box("a narrow container", "<div style=\"width:40px\">"),
        in_box("a wide container", "<div style=\"width:100px\">"),
    ]);
}
