//! A paragraph of one line whose own bottom padding or border crosses the end
//! of a page. Nothing breaks between the line and those edges, so the page
//! breaks before the paragraph when content comes before it, and the whole
//! paragraph starts the next page; its edges go on to the pages after it when
//! it is taller than a page. A paragraph at the top of a page stays, and its
//! edges continue on the next page. Floats that put the line down below the
//! top of the content box do not change that. A paragraph of more lines keeps
//! breaking between its lines. Where each line falls is what Chromium prints
//! for the same documents on pages 100px square, with Ahem glyphs of 20px on
//! lines of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lays out `body` on the pages and with the font of the Chromium documents.
fn lay_out(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
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

/// The page, glyph and glyph top of every word of text in `body`.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let result = lay_out(body);
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for word in run.text.split_whitespace() {
                let glyph = word.chars().next().unwrap();
                tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
            }
        }
    }
    tops.sort_unstable();
    tops
}

/// A document and the page, glyph and top of each of its words.
type Row = (&'static str, &'static [(u32, char, i32)]);

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}

#[test]
fn a_paragraph_of_one_line_moves_with_its_own_edges() {
    assert_rows(&[
        (
            "<div style='padding-bottom:0px'><div style='height:20px'></div><p style='margin:0;width:20px;padding-bottom:60px'>A</p></div><div>Z</div>",
            &[(1, 'A', 5), (2, 'Z', 5)],
        ),
        (
            "<div style='padding-bottom:0px'><div style='height:40px'></div><p style='margin:0;width:20px;border-bottom:40px solid'>A</p></div><div>Z</div>",
            &[(1, 'A', 5), (1, 'Z', 75)],
        ),
        (
            "<div style='padding-bottom:20px'><div style='height:50px'></div><div style='padding-bottom:0'><p style='margin:0;width:20px;padding-bottom:25px;border-bottom:25px solid'>A</p></div></div><div>Z</div>",
            &[(1, 'A', 5), (2, 'Z', 5)],
        ),
        (
            "<div style='padding-bottom:0px'><div style='height:10px'></div><p style='margin:0;width:20px;padding-bottom:100px'>A</p></div><div>Z</div>",
            &[(1, 'A', 5), (3, 'Z', 5)],
        ),
    ]);
}

#[test]
fn a_paragraph_of_one_line_stays_when_its_edges_fit_or_it_starts_the_page() {
    assert_rows(&[
        (
            "<div style='padding-bottom:0px'><p style='margin:0;width:20px;padding-bottom:100px'>A</p></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'Z', 5)],
        ),
        (
            "<div style='padding-bottom:0px'><div style='height:40px'></div><p style='margin:0;width:20px;padding-bottom:30px'>A</p></div><div>Z</div>",
            &[(0, 'A', 45), (1, 'Z', 5)],
        ),
        (
            "<div style='padding-bottom:0px'><div style='height:10px'></div><p style='margin:0;width:20px;padding-bottom:60px'>A</p></div><div>Z</div>",
            &[(0, 'A', 15), (1, 'Z', 5)],
        ),
    ]);
}

#[test]
fn a_paragraph_whose_line_a_float_puts_down_moves_with_its_own_edges() {
    assert_rows(&[
        (
            "<div style='height:20px'></div><div style='float:left;width:10px;height:10px'></div><p style='margin:0 0 0px;width:20px;padding-top:0px;padding-bottom:60px'>A</p><div>Z</div>",
            &[(1, 'A', 5), (2, 'Z', 5)],
        ),
        (
            "<div style='height:0px'></div><div>Y</div><div style='float:left;width:10px;height:10px'></div><p style='margin:0 0 20px;width:20px;padding-top:0px;padding-bottom:40px'>A</p><div>Z</div>",
            &[(0, 'Y', 5), (1, 'A', 5), (2, 'Z', 5)],
        ),
    ]);
}

#[test]
fn a_paragraph_of_more_lines_breaks_between_them() {
    assert_rows(&[
        (
            "<div style='padding-bottom:0px'><div style='height:40px'></div><p style='margin:0;width:20px;padding-bottom:40px'>A B</p></div><div>Z</div>",
            &[(0, 'A', 45), (1, 'B', 5), (1, 'Z', 75)],
        ),
        (
            "<div style='padding-bottom:40px'><div style='height:40px'></div><p style='margin:0;width:20px;padding-bottom:20px'>A B C</p></div><div>Z</div>",
            &[(0, 'A', 45), (0, 'B', 75), (1, 'C', 5), (2, 'Z', 5)],
        ),
    ]);
}
