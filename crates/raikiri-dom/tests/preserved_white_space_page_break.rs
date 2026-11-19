//! A line of preserved white space after a box with a forced page break
//! after it is content of the flow, as Chromium prints it: the break falls
//! before the line, which starts the next page.
//!
//! The expected places are the ones Chromium gives the same markup when it
//! prints a 200px by 100px page with no margin and `10px/10px Ahem` text.

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
         @page a {{ size: 200px 100px; margin: 0 }} \
         html, body {{ margin: 0 }} body {{ font: 10px/10px Ahem }}</style>{body}"
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

#[test]
fn a_preserved_white_space_line_starts_the_next_page() {
    for (style, text, x_top) in [
        ("pre", "&#10;", 10.0),
        ("pre", "   ", 10.0),
        ("pre", "&#10;&#10;", 20.0),
        ("pre-line", "&#10;", 10.0),
        ("pre-wrap", "  ", 10.0),
        // Spaces that collapse take no line.
        ("pre-line", "   ", 0.0),
    ] {
        check(
            &format!(
                "<div style='white-space:{style}'><div style='break-after:page'>B</div>{text}</div>\
                 <div>X</div>"
            ),
            &[(0, "B", 0.0, 0.0), (1, "X", 0.0, x_top)],
        );
    }
}

#[test]
fn an_empty_preserved_line_before_text_moves_with_it() {
    check(
        "<div style='white-space:pre'><div style='break-after:page'>B</div>&#10;Q</div>\
         <div>X</div>",
        &[(0, "B", 0.0, 0.0), (1, "Q", 0.0, 10.0), (1, "X", 0.0, 20.0)],
    );
}

#[test]
fn a_preserved_white_space_line_in_a_cell_splits_the_row() {
    check(
        "<table style='border-spacing:0'><tr>\
         <td style='white-space:pre;padding:0'><div style='height:40px'></div>\
         <div style='break-after:page'>B</div>&#10;</td><td style='padding:0'>T</td></tr>\
         <tr><td style='padding:0'>R</td></tr></table>",
        &[(0, "B", 0.0, 40.0), (1, "R", 0.0, 10.0)],
    );
}

#[test]
fn a_preserved_white_space_line_takes_the_page_name_of_its_parent() {
    // The line between two `page: a` boxes has the parent's page name, so the
    // boxes do not share a page: three pages in all.
    check(
        "<div style='white-space:pre'><div style='page:a'>B</div>&#10;\
         <div style='page:a'>X</div></div>",
        &[(0, "B", 0.0, 0.0), (2, "X", 0.0, 0.0)],
    );
}

#[test]
fn white_space_between_flex_items_takes_no_forced_break() {
    // The newline lays out no line between the items, so the margin of B is
    // what moves it down the next page.
    check(
        "<div style='display:flex;flex-direction:column;white-space:pre'>\
         <div style='break-after:page'>A</div>&#10;<div style='margin-top:10px'>B</div></div>",
        &[(0, "A", 0.0, 0.0), (1, "B", 0.0, 10.0)],
    );
}

#[test]
fn leading_preserved_white_space_keeps_the_first_page_unnamed() {
    // The line of spaces is content of the body's page, so the first page
    // is unnamed and the named box starts the second.
    check(
        "<body style='white-space:pre'>  <div style='page:a'>A</div>",
        &[(1, "A", 0.0, 0.0)],
    );
}
