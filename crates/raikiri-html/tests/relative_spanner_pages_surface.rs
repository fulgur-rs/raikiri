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

/// The number of pages `body` prints on 100x100 pages with no margin and
/// 10px Ahem lines 10px apart.
fn pages(body: &str) -> u32 {
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
    result.page_count()
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
