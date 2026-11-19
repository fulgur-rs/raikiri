//! Forced page breaks at the edges of a table cell. A forced break before
//! the first box of a cell's block flow, or after its last one, with only
//! floats, positioned boxes or empty `display: contents` boxes beside it,
//! lies at the edge of the cell rather than inside it: Chromium breaks the
//! page before or after the whole row instead, and not at all when the row
//! starts the first page. Where each glyph falls is what Chromium prints for
//! the same documents on pages 200px wide and 100px tall, with Ahem lines of
//! 10px.

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

/// A forced break before the first box of a cell moves the whole row to the
/// next page, from any cell, through nested blocks and past floats.
#[test]
fn a_break_at_the_start_of_a_cell_moves_its_row() {
    assert_rows(&[
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'><div style='break-before:page'>B</div></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[(0, 'P', 0), (1, 'B', 50), (1, 'T', 0), (1, 'X', 60)],
        ),
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:0'><div style='height:60px'>T</div></td><td style='padding:0;vertical-align:bottom'><div style='break-before:page'>B</div></td></tr></table><div>X</div>",
            &[(0, 'P', 0), (1, 'B', 50), (1, 'T', 0), (1, 'X', 60)],
        ),
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:5px;vertical-align:top'><div><div style='break-before:page'>B</div>C</div></td><td style='padding:0'><div style='height:30px'>T</div></td></tr></table><div>X</div>",
            &[
                (0, 'P', 0),
                (1, 'B', 5),
                (1, 'C', 15),
                (1, 'T', 0),
                (1, 'X', 30),
            ],
        ),
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:0;vertical-align:middle'><div style='break-before:page'>A</div></td><td style='padding:0'><div style='height:60px'>F</div></td></tr></table><div>X</div>",
            &[(0, 'P', 0), (1, 'A', 25), (1, 'F', 0), (1, 'X', 60)],
        ),
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'><div style='float:right;width:10px'>Q</div><div style='break-before:page'>B</div></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[
                (0, 'P', 0),
                (1, 'B', 50),
                (1, 'Q', 40),
                (1, 'T', 0),
                (1, 'X', 60),
            ],
        ),
    ]);
}

/// The same break in a row that starts the first page breaks nothing, and
/// the cell keeps its alignment.
#[test]
fn a_break_at_the_start_of_a_cell_at_the_top_of_the_first_page_does_nothing() {
    assert_rows(&[
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'><div style='break-before:page'>B</div></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[(0, 'B', 50), (0, 'T', 0), (0, 'X', 60)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom;position:relative'><div style='position:absolute;top:0;left:100px'>Q</div><div style='break-before:page'>B</div></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[(0, 'B', 50), (0, 'Q', 0), (0, 'T', 0), (0, 'X', 60)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'><span style='display:contents'></span><div style='break-before:page'>A</div></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[(0, 'A', 50), (0, 'T', 0), (0, 'X', 60)],
        ),
    ]);
}

/// A forced break after the last box of a cell breaks the page after the
/// row, from any cell; one in a cell that spans rows breaks after the row it
/// starts in.
#[test]
fn a_break_at_the_end_of_a_cell_breaks_after_its_row() {
    assert_rows(&[
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'><div style='break-after:page'>B</div></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[(0, 'B', 50), (0, 'T', 0), (1, 'X', 0)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'><div style='break-after:page'>B</div></td><td style='padding:0'><div style='height:30px'>T</div></td></tr><tr><td>R</td></tr></table><div>X</div>",
            &[(0, 'B', 20), (0, 'T', 0), (1, 'R', 1), (1, 'X', 12)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0'><div style='height:30px'>T</div></td><td style='padding:0;vertical-align:bottom'><div style='break-after:page'>B</div></td></tr><tr><td>R</td></tr></table><div>X</div>",
            &[(0, 'B', 20), (0, 'T', 0), (1, 'R', 1), (1, 'X', 12)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0'>P</td><td rowspan='2' style='padding:0'><div style='break-after:page'>S</div></td></tr><tr><td style='padding:0'>B</td></tr><tr><td>R</td></tr></table><div>X</div>",
            &[(0, 'P', 0), (1, 'B', 0), (1, 'R', 11), (1, 'X', 22)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'><div style='break-after:page'>A</div><span style='display:contents'></span></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[(0, 'A', 50), (0, 'T', 0), (1, 'X', 0)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:middle;position:relative'><div style='break-after:page'>B</div><div style='position:absolute;top:0;left:100px'>Q</div></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[(0, 'B', 25), (0, 'Q', 0), (0, 'T', 0), (1, 'X', 0)],
        ),
    ]);
}

/// Generated content before the first box of a cell is content of the cell,
/// so a break after it falls inside the cell, which splits there and leaves
/// the cells beside it aligned within the row's part on the page. A cell
/// with no row element of its own keeps its break too.
#[test]
fn a_break_after_generated_content_or_in_a_lone_cell_stays_in_the_cell() {
    assert_rows(&[
        (
            "<style>td.g::before{content:'L';display:block}</style><div>P</div><table style='border-spacing:0'><tr><td class='g' style='padding:0'><div style='break-before:page'>B</div></td><td style='padding:0'>T</td></tr></table><div>X</div>",
            &[
                (0, 'L', 10),
                (0, 'P', 0),
                (0, 'T', 50),
                (1, 'B', 0),
                (1, 'X', 10),
            ],
        ),
        (
            "<style>td.g::before{content:'L';display:block}</style><table style='border-spacing:0'><tr><td class='g' style='padding:0'><div style='break-before:page'>B</div></td><td style='padding:0'>T</td></tr></table><div>X</div>",
            &[(0, 'L', 0), (0, 'T', 45), (1, 'B', 0), (1, 'X', 10)],
        ),
        (
            "<div>P</div><div style='display:table-cell'><div style='break-before:page'>B</div></div><div>X</div>",
            &[(0, 'P', 0), (1, 'B', 0), (1, 'X', 10)],
        ),
    ]);
}

/// Generated content keeps a break inside the cell only on its own side: a
/// cell's `::before` does not keep a break after its last box inside it, and
/// its `::after` does not keep a break before its first box.
#[test]
fn generated_content_on_the_other_side_leaves_the_break_at_the_edge() {
    assert_rows(&[
        (
            "<style>td.g::before{content:'L';display:block}</style><table style='border-spacing:0'><tr><td class='g' style='padding:0;vertical-align:bottom'><div style='break-after:page'>B</div></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[(0, 'B', 50), (0, 'L', 40), (0, 'T', 0), (1, 'X', 0)],
        ),
        (
            "<style>td.g::after{content:'L';display:block}</style><div>P</div><table style='border-spacing:0'><tr><td class='g' style='padding:0;vertical-align:bottom'><div style='break-before:page'>B</div></td><td style='padding:0'><div style='height:60px'>T</div></td></tr></table><div>X</div>",
            &[
                (0, 'P', 0),
                (1, 'B', 40),
                (1, 'L', 50),
                (1, 'T', 0),
                (1, 'X', 60),
            ],
        ),
    ]);
}

/// A box that pagination does not break through, such as one with size
/// containment, keeps the breaks inside it from the row.
#[test]
fn a_break_inside_a_monolithic_box_does_not_reach_the_row() {
    assert_rows(&[
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:0'><div style='contain:size;width:20px;height:20px'><div style='break-before:page'>B</div></div></td><td style='padding:0'>T</td></tr></table><div>X</div>",
            &[(0, 'B', 10), (0, 'P', 0), (0, 'T', 15), (0, 'X', 30)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0'><div style='contain:size;width:20px;height:20px'><div style='break-after:page'>B</div></div></td><td style='padding:0'>T</td></tr><tr><td>R</td></tr></table><div>X</div>",
            &[(0, 'B', 0), (0, 'R', 21), (0, 'T', 5), (0, 'X', 32)],
        ),
    ]);
}

/// A cell in an orthogonal writing mode is not broken across pages, so the
/// breaks at the edges of its block flow stay inside it as well. The
/// vertical glyph is left out: its position is not comparable.
#[test]
fn a_break_in_an_orthogonal_cell_does_not_reach_the_row() {
    assert_rows(&[
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:0;writing-mode:vertical-rl'><div style='break-before:page'>B</div></td><td style='padding:0'>T</td></tr></table><div>X</div>",
            &[(0, 'P', 0), (0, 'T', 10), (0, 'X', 20)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;writing-mode:vertical-rl'><div style='break-after:page'>B</div></td><td style='padding:0'>T</td></tr><tr><td>R</td></tr></table><div>X</div>",
            &[(0, 'R', 11), (0, 'T', 0), (0, 'X', 22)],
        ),
    ]);
}

/// White space that the style preserves makes a line before the first block
/// of the cell, so a break before that block falls inside the cell, which
/// Chromium breaks there and stretches to the end of the page.
#[test]
fn preserved_white_space_keeps_the_break_inside_the_cell() {
    assert_rows(&[
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:0;white-space:pre'>&#10;<div style='break-before:page'>B</div></td><td style='padding:0'>T</td></tr></table><div>X</div>",
            &[(0, 'P', 0), (0, 'T', 50), (1, 'B', 0), (1, 'X', 10)],
        ),
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:0;white-space:pre-wrap'>  <div style='break-before:page'>B</div></td><td style='padding:0'>T</td></tr></table><div>X</div>",
            &[(0, 'P', 0), (0, 'T', 50), (1, 'B', 0), (1, 'X', 10)],
        ),
        (
            "<div>P</div><table style='border-spacing:0'><tr><td style='padding:0'><div style='white-space:pre'>&#10;<div style='break-before:page'>B</div></div></td><td style='padding:0'>T</td></tr></table><div>X</div>",
            &[(0, 'P', 0), (1, 'B', 0), (1, 'X', 10)],
        ),
    ]);
}
