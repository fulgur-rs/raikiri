//! The outside marker of a list item sits beside the first line box of the
//! item, which can come after blocks with no line of their own, and on the page
//! that holds it. The positions are the ones Chromium prints for the same
//! documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// The page, text and glyph top of every run of a list whose items are
/// `items`, on 100px pages with 20px Ahem on 30px lines. The marker is the
/// string `M`.
fn runs(items: &str) -> Vec<(u32, String, f32)> {
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
         body{{margin:0;font:20px/30px Ahem}}\
         ul{{margin:0;padding:0 0 0 40px;list-style:'M'}}</style><body><ul>{items}</ul></body>"
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
    let mut runs = Vec::new();
    for index in 0..6 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            runs.push((index, run.text.trim_end().to_owned(), run.origin.1 - ASCENT));
        }
    }
    runs
}

fn spacer(height: u32) -> String {
    format!("<div style='height:{height}px'></div>")
}

#[test]
fn the_marker_is_beside_the_first_line_after_an_empty_block() {
    assert_eq!(
        runs(&format!("<li>{}z</li>", spacer(65))),
        [(0, "M".into(), 70.0), (0, "z".into(), 70.0)]
    );
}

#[test]
fn the_marker_passes_over_every_block_with_no_line() {
    let expected = |top: f32| [(0, "M".to_owned(), top), (0, "z".to_owned(), top)];
    // Two blocks before the line.
    assert_eq!(
        runs(&format!("<li>{}{}z</li>", spacer(30), spacer(20))),
        expected(55.0)
    );
    // A float and a block.
    assert_eq!(
        runs(&format!(
            "<li><div style='height:10px;width:10px;float:left'></div>{}z</li>",
            spacer(30)
        )),
        expected(35.0)
    );
}

#[test]
fn the_marker_is_beside_the_first_line_of_a_block_after_an_empty_one() {
    let items = format!("<li>{}<div>y</div></li>", spacer(30));
    assert_eq!(runs(&items), [(0, "M".into(), 35.0), (0, "y".into(), 35.0)]);
    let items = format!(
        "<li>{}<div style='display:flex'><span>y</span></div></li>",
        spacer(30)
    );
    assert_eq!(runs(&items), [(0, "M".into(), 35.0), (0, "y".into(), 35.0)]);
}

#[test]
fn an_item_with_no_line_keeps_the_marker_at_its_top() {
    assert_eq!(
        runs(&format!("<li>{}</li><li>q</li>", spacer(30))),
        [
            (0, "M".into(), 5.0),
            (0, "M".into(), 35.0),
            (0, "q".into(), 35.0)
        ]
    );
}

#[test]
fn the_marker_goes_to_the_page_of_the_first_line() {
    // The line would cross the end of the page after a block of 85px, and
    // starts the next one after 100px.
    for height in [85, 100] {
        assert_eq!(
            runs(&format!("<li>{}z</li>", spacer(height))),
            [(1, "M".into(), 5.0), (1, "z".into(), 5.0)],
            "{height}px block"
        );
    }
}

#[test]
fn the_marker_stays_on_the_first_page_with_a_first_line_on_it() {
    assert_eq!(
        runs(&format!(
            "<li><div style='height:30px'>y</div>{}z</li>",
            spacer(50)
        )),
        [
            (0, "M".into(), 5.0),
            (0, "y".into(), 5.0),
            (1, "z".into(), 5.0)
        ]
    );
}
