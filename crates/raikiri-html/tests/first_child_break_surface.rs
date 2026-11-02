//! A forced break before the first child of a block. A page does not break
//! between the start of a block and its first child, so the break is the one
//! before the block: it shares the break that already opened the page, and the
//! block keeps its padding, border and margin on the new page. The positions
//! are the ones Chromium prints for the same documents (`page.pdf` with the
//! page size taken from `@page`, the text read from the rendered pages).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// A forced break after a 30px block.
const BREAK: &str = "<div style='break-after:page;height:30px'>a</div>";
/// Two lines of two words.
const CHILD: &str = "<div style='break-before:page'>XX XX XX XX</div>";

/// The number of pages, and the page and top of every line of text but the
/// lead-in `a`. A 20px glyph in a 30px line sits 5px below the line.
fn pages(body: &str) -> (usize, Vec<(u32, f32)>) {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let mut count = 0;
    let mut lines = Vec::new();
    for index in 0..6 {
        let Some(page) = result.page(index) else {
            continue;
        };
        count += 1;
        for run in page.text_runs() {
            if run.text.trim_end() != "a" {
                assert_eq!(run.origin.0, 0.0);
                lines.push((index, run.origin.1 - ASCENT));
            }
        }
    }
    (count, lines)
}

/// The text of `CHILD` on the second page, its first line `top` down the page.
fn on_second_page(top: f32) -> (usize, Vec<(u32, f32)>) {
    (2, vec![(1, top), (1, top + 30.0)])
}

#[test]
fn the_edges_of_the_block_stay_with_the_child_on_the_page_the_break_opened() {
    // The break before the child does not open a page of its own after the
    // one the block before the wrapper forced: the wrapper's padding, border
    // and margin stay above the child.
    for (wrapper, top) in [
        ("", 5.0),
        ("padding-top:5px", 10.0),
        ("border-top:5px solid red", 10.0),
        ("margin-top:10px", 15.0),
        ("padding-top:5px;margin-top:10px", 20.0),
        ("overflow:hidden;padding-top:5px", 10.0),
    ] {
        let body = format!("{BREAK}<div style='{wrapper}'>{CHILD}</div>");
        assert_eq!(pages(&body), on_second_page(top), "wrapper {wrapper:?}");
    }
}

#[test]
fn every_block_the_child_begins_keeps_its_edges() {
    let body = format!(
        "{BREAK}<div style='padding-top:5px'><div style='padding-top:7px'>{CHILD}</div></div>"
    );
    assert_eq!(pages(&body), on_second_page(17.0));
    // The margin of the wrapper collapses with the larger margin of the child.
    let body = format!(
        "{BREAK}<div style='margin-top:10px'>\
         <div style='margin-top:20px;break-before:page'>XX XX XX XX</div></div>"
    );
    assert_eq!(pages(&body), on_second_page(25.0));
    // A list item begins the list.
    let body = format!(
        "{BREAK}<ul style='margin:0;padding:5px 0 0 0;list-style:none'>\
         <li style='break-before:page'>XX XX XX XX</li></ul>"
    );
    assert_eq!(pages(&body), on_second_page(10.0));
}

#[test]
fn white_space_and_out_of_flow_boxes_do_not_come_before_the_child() {
    let body = format!("{BREAK}<div style='padding-top:5px'>\n  {CHILD}\n</div>");
    assert_eq!(pages(&body), on_second_page(10.0));
    let body = format!(
        "{BREAK}<div style='padding-top:5px'>\
         <div style='position:absolute;width:0;height:0'></div>{CHILD}</div>"
    );
    assert_eq!(pages(&body), on_second_page(10.0));
}

#[test]
fn a_box_or_text_before_the_child_keeps_its_break() {
    // The wrapper starts the second page with what comes before the child, and
    // the child begins the third.
    for before in ["<div style='height:10px'></div>", "<div></div>"] {
        let body = format!("{BREAK}<div style='padding-top:5px'>{before}{CHILD}</div>");
        assert_eq!(
            pages(&body),
            (3, vec![(2, 5.0), (2, 35.0)]),
            "before {before:?}"
        );
    }
    let body = format!("{BREAK}<div style='padding-top:5px'>b{CHILD}</div>");
    assert_eq!(pages(&body), (3, vec![(1, 10.0), (2, 5.0), (2, 35.0)]));
}

#[test]
fn a_break_before_the_first_box_of_the_document_opens_no_page() {
    // Nothing comes before the wrapper, so the break has no page to end.
    let nested =
        format!("<div style='padding-top:5px'><div style='padding-top:7px'>{CHILD}</div></div>");
    for (body, top) in [
        (format!("<div>{CHILD}</div>"), 5.0),
        (format!("<div style='padding-top:5px'>{CHILD}</div>"), 10.0),
        (nested, 17.0),
    ] {
        let expected = (1, vec![(0, top), (0, top + 30.0)]);
        assert_eq!(pages(&body), expected, "{body}");
    }
}
