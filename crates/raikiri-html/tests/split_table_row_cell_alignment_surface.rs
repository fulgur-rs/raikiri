//! The block-axis alignment of the cells of a table row that a page break
//! splits. Chromium lays such a cell out as if it were aligned to the top:
//! the room that `vertical-align: middle`, `bottom` or `baseline` puts above
//! its content stays below it, so the content starts on the page the row
//! starts on. A row that a forced break in one of its cells grows is laid
//! out at the height of its part on the page the break leaves instead: the
//! cells beside the breaking one keep their alignment within that part.
//! Where each glyph falls is what Chromium prints for the same
//! documents on pages 200px wide and 100px tall, with Ahem lines of 10px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph and glyph top of every line of text in `body`, where
/// the painter draws it.
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
            // A relative offset that layout leaves to paint (on a row, a row
            // group or a cell) moves the run by its transform.
            let top = (run.origin.0, run.origin.1 - run.ascent);
            let top = run
                .transform
                .map_or(top, |transform| transform.apply(top))
                .1;
            for glyph in run.text.chars().filter(|glyph| !glyph.is_whitespace()) {
                tops.push((index, glyph, top.round() as i32));
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

/// A row that ends on its page keeps the alignment of its cells, also
/// when a relative offset paints it past the page end.
#[test]
fn rows_that_fit_keep_their_alignment() {
    assert_rows(&[
        (
            "<table style='border-spacing:0;position:relative;top:50px'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 50), (0, 'B', 70), (0, 'X', 70), (1, 'C', 10)],
        ),
        (
            "<div style='position:relative;top:50px'><table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table></div><div id='X'>X</div>",
            &[(0, 'A', 50), (0, 'B', 70), (0, 'X', 70), (1, 'C', 10)],
        ),
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

/// An absolutely positioned box in a split cell keeps the static position the
/// table gave it, and a split row of a table in a split cell starts at its top
/// too.
#[test]
fn positioned_boxes_and_nested_split_rows_in_a_split_cell() {
    assert_rows(&[
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div><div id='Q' style='position:absolute'>Q</div></td></tr></table><div id='X'>X</div>",
            &[
                (0, 'A', 0),
                (0, 'B', 20),
                (0, 'C', 0),
                (0, 'Q', 10),
                (1, 'X', 70),
            ],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:150px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div><table style='border-spacing:0'><tr><td style='padding:0'><div id='G' style='height:20px'>G</div><div id='H' style='height:150px'>H</div></td><td style='padding:0;vertical-align:middle'><div id='K'>K</div></td></tr></table></td></tr></table><div id='X'>X</div>",
            &[
                (0, 'A', 0),
                (0, 'B', 20),
                (0, 'C', 0),
                (0, 'G', 10),
                (0, 'H', 30),
                (0, 'K', 10),
                (1, 'X', 80),
            ],
        ),
    ]);
}

/// A forced break in a cell grows its row past the page end. The cells beside
/// it keep a middle or bottom alignment within the part of the row on the
/// page the break leaves, which ends at the page end, inside their padding;
/// one aligned to the baseline starts at its top. This holds whatever side of
/// the breaking cell they are on, after another row, with border spacing, for
/// a break after a box, a break in a nested block, a moved box that keeps its
/// margin, and further breaks in the same cell.
#[test]
fn cells_beside_a_forced_break_align_within_the_rows_part_on_its_page() {
    assert_rows(&[
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 90), (1, 'X', 50)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 90), (1, 'X', 50)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:middle'><div id='C'>C</div></td><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 45), (1, 'X', 50)],
        ),
        (
            "<div style='height:30px'>P</div><table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 30), (0, 'C', 90), (0, 'P', 0), (1, 'X', 50)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td><td style='padding:5px;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 85), (1, 'X', 50)],
        ),
        (
            "<table style='border-spacing:4px'><tr><td style='padding:0'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 4), (0, 'C', 90), (1, 'X', 54)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C' style='height:40px'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 60), (1, 'X', 50)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div>A</div><div style='break-before:page'>B</div><div style='break-before:page'>D</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 90), (2, 'X', 10)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0'><div id='A' style='height:20px;break-after:page'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 90), (1, 'X', 50)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0'><div><div id='A' style='height:20px'>A</div><div style='padding-top:5px'><div id='B' style='break-before:page;height:50px'>B</div></div></div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 90), (1, 'B', 5), (1, 'X', 55)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;margin-top:15px;height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 90), (1, 'B', 15), (1, 'X', 65)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td><td style='padding:0;vertical-align:baseline'><div style='font-size:30px;line-height:30px'>G</div></td><td style='padding:0;vertical-align:baseline'><div>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 0), (0, 'G', 0), (1, 'X', 50)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td><td style='padding:0;vertical-align:middle'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 45), (1, 'X', 50)],
        ),
    ]);
}

/// A cell that a forced break splits itself starts at its top, whatever its
/// alignment, while the cells beside it align within the part on the page.
#[test]
fn a_cell_with_a_forced_break_of_its_own_starts_at_its_top() {
    assert_rows(&[
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:middle'><div id='D'>D</div><div id='E' style='break-before:page'>E</div></td><td style='padding:0'><div id='F' style='height:60px'>F</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'D', 0), (0, 'F', 20), (1, 'X', 10)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'><div id='D'>D</div><div id='E' style='break-before:page'>E</div></td><td style='padding:0'><div id='F' style='height:60px'>F</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'D', 0), (0, 'F', 20), (1, 'X', 10)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;height:50px'>B</div></td><td style='padding:0'><div id='D' style='height:40px'>D</div><div id='E' style='break-before:page'>E</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 90), (0, 'D', 0), (1, 'X', 50)],
        ),
    ]);
}

/// A relative offset on the box a forced break moves does not change where
/// the row's part on the page ends. A forced break inside an inline-block or
/// on an inline box does not split its cell, which keeps its alignment.
#[test]
fn relative_offsets_and_breaks_outside_the_cells_block_flow() {
    assert_rows(&[
        (
            "<table style='border-spacing:0'><tr><td style='padding:0'><div id='A' style='height:20px'>A</div><div id='B' style='break-before:page;position:relative;top:10px;height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 0), (0, 'C', 90), (1, 'B', 10), (1, 'X', 50)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:middle'><div id='A'>A</div><div style='display:inline-block'><div>Q</div><div style='break-before:page'>R</div></div></td><td style='padding:0'><div id='F' style='height:60px'>F</div></td></tr></table><div id='X'>X</div>",
            &[
                (0, 'A', 15),
                (0, 'F', 0),
                (0, 'Q', 25),
                (0, 'R', 35),
                (0, 'X', 60),
            ],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:bottom'>A<span style='break-before:page'>B</span></td><td style='padding:0'><div id='T' style='height:60px'>T</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 50), (0, 'B', 50), (0, 'T', 0), (0, 'X', 60)],
        ),
        (
            "<table style='border-spacing:0'><tr><td style='padding:0;vertical-align:middle'><span style='break-after:page'>A</span>B</td><td style='padding:0'><div id='T' style='height:60px'>T</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 25), (0, 'B', 25), (0, 'T', 0), (0, 'X', 60)],
        ),
    ]);
}

/// A row whose place in the flow crosses the page end is split there, and
/// its cells start at its top, also when a relative offset paints it above
/// that place, whether the offset is on the table, a box around it, its row
/// group or the row (Chromium splits at the flow place and paints the parts
/// moved). A cell's own offset moves only that cell, and the flow after the
/// table follows the row's place in the flow, not its painted one.
#[test]
fn rows_split_in_the_flow_and_moved_by_a_relative_offset() {
    assert_rows(&[
        (
            "<div style='height:40px'></div><table style='border-spacing:0;position:relative;top:-30px'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 10), (0, 'B', 30), (0, 'C', 10), (1, 'X', 10)],
        ),
        (
            "<div style='height:40px'></div><table style='border-spacing:0;position:relative;bottom:30px'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 10), (0, 'B', 30), (0, 'C', 10), (1, 'X', 10)],
        ),
        (
            "<div style='height:40px'></div><div style='position:relative;top:-30px'><table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table></div><div id='X'>X</div>",
            &[(0, 'A', 10), (0, 'B', 30), (0, 'C', 10), (1, 'X', 10)],
        ),
        (
            "<div style='height:40px'></div><table style='border-spacing:0'><tbody style='position:relative;top:-30px'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></tbody></table><div id='X'>X</div>",
            &[(0, 'A', 10), (0, 'B', 30), (0, 'C', 10), (1, 'X', 10)],
        ),
        (
            "<div style='height:40px'></div><table style='border-spacing:0'><tr style='position:relative;top:-30px'><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 10), (0, 'B', 30), (0, 'C', 10), (1, 'X', 10)],
        ),
        (
            "<div style='height:40px'></div><table style='border-spacing:0;position:relative;top:-30px'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:middle'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 10), (0, 'B', 30), (0, 'C', 10), (1, 'X', 10)],
        ),
        (
            "<div style='height:40px'></div><table style='border-spacing:0;position:relative;top:-50px'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'B', 10), (1, 'X', 10)],
        ),
        (
            "<div style='height:40px'></div><table style='border-spacing:0;position:relative;top:-30px'><tr><td style='padding:0'><div id='D' style='height:20px'>D</div></td></tr><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[
                (0, 'A', 30),
                (0, 'B', 50),
                (0, 'C', 30),
                (0, 'D', 10),
                (1, 'X', 30),
            ],
        ),
        (
            "<div style='height:40px'></div><table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom;position:relative;top:-30px'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 40), (0, 'B', 60), (0, 'C', 10), (1, 'X', 10)],
        ),
        (
            "<div style='height:40px'></div><table style='border-spacing:0'><tr><td style='padding:0;vertical-align:top;position:relative;top:-30px'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 10), (0, 'B', 30), (0, 'C', 40), (1, 'X', 10)],
        ),
        (
            "<div style='height:40px'></div><table style='border-spacing:0'><tr style='position:relative;top:20px'><td style='padding:0;vertical-align:top'><div id='A' style='height:20px'>A</div><div id='B' style='height:50px'>B</div></td><td style='padding:0;vertical-align:bottom'><div id='C'>C</div></td></tr></table><div id='X'>X</div>",
            &[(0, 'A', 60), (0, 'B', 80), (0, 'C', 60), (1, 'X', 10)],
        ),
    ]);
}
