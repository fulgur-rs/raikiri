//! A float that does not fit the rest of its page and cannot break inside
//! (`break-inside: avoid`, or a replaced box) moves whole to the next page.
//! Chromium then lays out the content beside it on the page it left as if
//! the float were not there: the lines and boxes take the whole width. The
//! content that follows the float onto the next page wraps beside it there.
//!
//! The expected places are the ones Chromium gives the same markup when it
//! prints a 200px by 100px page with no margin and `10px/20px Ahem` text.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The `(page, text, x, glyph top)` of every text run, page by page.
fn runs(body: &str) -> Vec<(usize, String, f32, f32)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 200px 100px; margin: 0 }} \
         html, body {{ margin: 0 }} body {{ font: 10px/20px Ahem }}</style>\
         <div style='height:50px'>W</div>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    let mut runs = Vec::new();
    for (index, page) in result.pages().enumerate() {
        for run in page.text_runs() {
            let text = run.text.trim();
            if !text.is_empty() {
                // The Ahem ascent is 8px of the 10px em.
                runs.push((index, text.to_owned(), run.origin.0, run.origin.1 - 8.0));
            }
        }
    }
    runs
}

fn check(body: &str, expected: &[(usize, &str, f32, f32)]) {
    let runs = runs(body);
    for &(page, text, x, y) in expected {
        let found = runs.iter().find(|(_, run, ..)| run.starts_with(text));
        let Some((got_page, _, got_x, got_y)) = found else {
            panic!("{body}: no run {text:?} in {runs:?}");
        };
        assert!(
            *got_page == page && (got_x - x).abs() <= 0.04 && (got_y - y).abs() <= 0.04,
            "{body}: {text:?} is on page {got_page} at ({got_x}, {got_y}), \
             expected page {page} at ({x}, {y}); all runs: {runs:?}"
        );
    }
}

const AVOID: &str = "float:left;height:70px;break-inside:avoid";

#[test]
fn lines_beside_a_moved_float_take_the_whole_width() {
    check(
        &format!("<div><div style='{AVOID};width:30px'>Y</div>QQ QQ QQ QQ</div><div>X</div>"),
        &[
            (0, "QQ", 0.0, 55.0),
            (0, "X", 0.0, 75.0),
            (1, "Y", 0.0, 5.0),
        ],
    );
}

#[test]
fn a_block_after_a_moved_float_takes_the_whole_width() {
    check(
        &format!("<div style='{AVOID}'>Y</div><div>X</div>"),
        &[(0, "X", 0.0, 55.0), (1, "Y", 0.0, 5.0)],
    );
    check(
        &format!(
            "<div><div style='{AVOID};width:20px'>Y</div>\
             <p style='margin:0'>Q</p><p style='margin:0'>R</p></div><div>X</div>"
        ),
        &[
            (0, "Q", 0.0, 55.0),
            (0, "R", 0.0, 75.0),
            (1, "Y", 0.0, 5.0),
            (1, "X", 20.0, 5.0),
        ],
    );
}

#[test]
fn a_moved_replaced_float_frees_the_lines_too() {
    check(
        "<div><img style='float:left;width:30px;height:70px'>Q</div><div>X</div>",
        &[(0, "Q", 0.0, 55.0), (0, "X", 0.0, 75.0)],
    );
}

#[test]
fn a_line_crossing_the_page_end_wraps_beside_the_moved_float() {
    check(
        &format!("<div><div style='{AVOID}'>Y</div>Q<br>R<br>S<br>T</div><div>X</div>"),
        &[
            (0, "Q", 0.0, 55.0),
            (0, "R", 0.0, 75.0),
            (1, "Y", 0.0, 5.0),
            (1, "S", 10.0, 5.0),
            (1, "T", 10.0, 25.0),
            (1, "X", 10.0, 45.0),
        ],
    );
}

#[test]
fn a_moved_right_float_leaves_the_lines_where_they_were() {
    check(
        "<div><div style='float:right;width:30px;height:70px;break-inside:avoid'>Y</div>\
         <div style='text-align:right'>Q</div></div><div style='text-align:right'>X</div>",
        &[
            (0, "Q", 190.0, 55.0),
            (0, "X", 190.0, 75.0),
            (1, "Y", 170.0, 5.0),
        ],
    );
}

#[test]
fn a_moved_float_in_a_relative_inline_takes_its_offset_once() {
    check(
        &format!(
            "<div><span style='position:relative;top:10px'>\
             <div style='{AVOID};width:30px'>Y</div>Q</span> R<br>S<br>T</div><div>X</div>"
        ),
        &[
            (0, "Q", 0.0, 65.0),
            (0, "S", 0.0, 75.0),
            (1, "Y", 0.0, 15.0),
            (1, "T", 30.0, 5.0),
            (1, "X", 30.0, 25.0),
        ],
    );
}
