//! How many pages a multicol container prints when a `column-span: all` box
//! in it is `position: relative` with a block offset. The columns place the
//! spanner without that offset, which the painter adds, so pagination has to
//! be told to measure it; a box moved past the page end makes another page.
//! The page counts are the ones Chromium prints for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// `body` laid out on 100x100 pages with no margin and 10px Ahem lines 10px
/// apart.
fn lay_out(body: &str) -> raikiri_html::DocumentLayout {
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
         body{{margin:0;font:10px/10px Ahem}}</style><body>{body}</body>"
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
    result
}

/// The number of pages `body` prints.
fn pages(body: &str) -> u32 {
    lay_out(body).page_count()
}

#[test]
fn a_relative_spanner_moved_past_the_page_end_makes_another_page() {
    let columns = "columns:2;column-gap:0;width:100px";
    let cases = [
        (
            "<div style='column-span:all;height:20px;position:relative;top:150px'>S</div>",
            "",
            2,
        ),
        (
            "<div>A</div><div style='column-span:all;position:relative;top:150px'>S</div>\
             <div>C</div>",
            "",
            2,
        ),
        (
            "<div>A</div><div style='column-span:all;position:relative;bottom:-150px'>S</div>\
             <div>C</div>",
            "",
            2,
        ),
        (
            "<div>A</div><div style='column-span:all;position:relative;top:150px'>\
             <div>S</div><div>T</div></div><div>C</div>",
            "",
            2,
        ),
        (
            "<div>A</div><div style='column-span:all;position:relative;top:150px'>S</div>\
             <div>C</div>",
            ";height:50px",
            2,
        ),
        (
            "<div>A</div><div style='column-span:all;position:relative;top:150px;\
             break-before:page'>S</div><div>C</div>",
            "",
            3,
        ),
        // An offset that stays on the page, or moves the box up, adds none.
        (
            "<div>A</div><div style='column-span:all;position:relative;top:50px'>S</div>\
             <div>C</div>",
            "",
            1,
        ),
        (
            "<div>A</div><div style='column-span:all;position:relative;top:-5px'>S</div>\
             <div>C</div>",
            "",
            1,
        ),
    ];
    for (content, style, expected) in cases {
        assert_eq!(
            pages(&format!("<div style='{columns}{style}'>{content}</div>")),
            expected,
            "{content}{style}"
        );
    }
}

#[test]
fn a_relative_spanner_on_its_page_is_painted_at_its_offset_once() {
    // Chromium prints S 60px and 5px down: the line before it, then the
    // offset, added once.
    for (top, expected) in [(50, 60.0), (-5, 5.0)] {
        let result = lay_out(&format!(
            "<div style='columns:2;column-gap:0;width:100px'><div>A</div>\
             <div style='column-span:all;position:relative;top:{top}px'>S</div>\
             <div>C</div></div>"
        ));
        let page = result.page(0).expect("a page");
        let run = page
            .text_runs()
            .into_iter()
            .find(|run| run.text.trim() == "S")
            .expect("the spanner's line");
        // Ahem's ascent puts the top of a 10px glyph 8px above the baseline.
        assert_eq!((run.origin.1 - 8.0).round(), expected, "{top}");
    }
}
