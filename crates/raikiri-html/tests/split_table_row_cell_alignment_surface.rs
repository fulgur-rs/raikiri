//! The block-axis alignment of the cells of a table row that a page break
//! splits. Chromium lays such a cell out as if it were aligned to the top:
//! the room that `vertical-align: middle`, `bottom` or `baseline` puts above
//! its content stays below it, so the content starts on the page the row
//! starts on. Where each glyph falls is what Chromium prints for the same
//! documents on pages 200px wide and 100px tall, with Ahem lines of 10px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph and glyph top of every line of text in `body`.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}} body{{margin:0;font:10px/10px Ahem}}</style><body>{body}</body>"
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
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for glyph in run.text.chars().filter(|glyph| !glyph.is_whitespace()) {
                tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
            }
        }
    }
    tops.sort_unstable();
    tops.dedup();
    tops
}

/// A document and the page, glyph and top of each glyph Chromium shows of it.
type Row = (&'static str, &'static [(u32, char, i32)]);

/// Compares the glyphs that `expected` names; a glyph it leaves out is
/// one the Chromium print does not report in a way that can be compared.
fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        let actual: Vec<_> = glyph_tops(body)
            .into_iter()
            .filter(|(_, glyph, _)| expected.iter().any(|(_, named, _)| named == glyph))
            .collect();
        assert_eq!(actual, expected, "{body}");
    }
}

/// Cells aligned to the bottom or the middle of a row that a page break splits
/// start at the top of the row, with their padding, under separated and
/// collapsed borders, as text directly in the cell and in nested blocks.
#[test]
fn bottom_and_middle_cells_of_a_split_row_start_at_its_top() {
    assert_rows(&[
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 0), (1, 'X', 70)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:middle'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 0), (1, 'X', 70)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:bottom'>C</td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 0), (1, 'X', 70)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:middle'>C</td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 0), (1, 'X', 70)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:5px;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 5), (1, 'X', 70)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0 0 30px 0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 0), (1, 'X', 70)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div><div id='D'>D</div></td></tr></table><div id='X'>X</div>",
            &[
                (0, 'A', 0),
                (0, 'B', 20),
                (0, 'C', 0),
                (0, 'D', 10),
                (1, 'X', 70),
            ],
        ),
        (
            "<div id='W'><table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'><div id='D'>D</div></div></td></tr></table></div><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'D', 0), (1, 'X', 70)],
        ),
        (
            "<table style='border-collapse:collapse'><tr><td style='border:4px solid;padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='border:4px solid;padding:0;vertical-align:middle'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 4), (0, 'B', 24), (0, 'C', 4), (1, 'X', 78)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:bottom;height:170px'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 0), (1, 'X', 70)],
        ),
    ]);
}

/// A cell aligned to the baseline of a split row drops that alignment too.
#[test]
fn baseline_cells_of_a_split_row_start_at_its_top() {
    assert_rows(&[(
        "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:baseline'><div id='A' style='font-size:30px;line-height:30px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:baseline'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
        &[(0, 'A', 0), (0, 'B', 30), (0, 'C', 0), (1, 'X', 80)],
    )]);
}

/// The row may follow another, run over more than one page, or hold a cell
/// that spans the next row too, or be split by a forced break past the
/// height of a sized cell.
#[test]
fn split_rows_after_others_taller_than_a_page_or_spanned_start_at_their_top() {
    assert_rows(&[
        (
            "<table style='border-spacing:0'><tr><td><div id='P' style='height:30px'>P</div></td></tr><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:middle'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[
                (0, 'A', 32),
                (0, 'B', 52),
                (0, 'C', 32),
                (0, 'P', 1),
                (2, 'X', 2),
            ],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:250px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 0), (2, 'X', 70)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td rowspan=2 style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr><tr><td><div id='E'>E</div></td></tr></table><div id='X'>X</div>",
            &[
                (0, 'A', 0),
                (0, 'B', 20),
                (0, 'C', 0),
                (1, 'E', 71),
                (1, 'X', 82),
            ],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top;height:150px'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 0), (1, 'X', 50)],
        ),
    ]);
}

/// A row that ends on its page keeps the alignment of its cells.
#[test]
fn rows_that_fit_keep_their_alignment() {
    assert_rows(&[
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 60), (0, 'X', 70)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:middle'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'B', 20), (0, 'C', 30), (0, 'X', 70)],
        ),
    ]);
}
