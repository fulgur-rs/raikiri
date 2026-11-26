//! A table row that crosses the page end breaks there with its cells, as
//! Chromium does, also in a table with a header or footer group: the lines
//! a cell moves to the next page go below the copy of a header the table
//! repeats, the page the row leaves ends above the copy of a repeated
//! footer, and a group that does not repeat (one taller than a quarter of
//! the page) leaves the rows to break as in a table without it. A row taller
//! than a page whose cell cannot put its first content before the page end
//! starts the next page whole, below the header's copy, or with the header
//! itself when it is the first row.
//!
//! The positions are the ones Chromium prints for the same documents
//! (`page.pdf`, 100px square pages). Each page lists the 20px Ahem glyphs on
//! it as `glyph@x,top`, with the top of a 30px line; the 20px lines of the
//! repeated groups read 5px above their top (`H@0,-5` is a header copy at the
//! top of the page). Chromium paints the header copies too, although its PDF
//! text lists only the first.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Each page as its width, then each 20px Ahem glyph on it as
/// `glyph@x,top`, sorted.
fn pages(body: &str) -> Vec<String> {
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
         body{{margin:0;font:20px/30px Ahem}}</style>{body}"
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
        .pages()
        .map(|page| {
            let mut glyphs = Vec::new();
            for run in page.text_runs() {
                // The 20px Ahem baseline sits 21px below the top of its 30px line.
                let top = (run.origin.1 - 21.0).round() as i32;
                let mut x = run.origin.0.round() as i32;
                for glyph in run.text.chars() {
                    if glyph != ' ' {
                        glyphs.push(format!("{glyph}@{x},{top}"));
                    }
                    x += 20;
                }
            }
            glyphs.sort();
            let width = page.geometry().page_box.width.round() as i32;
            format!("{width}: {}", glyphs.join(" "))
        })
        .collect()
}

fn expect(body: &str, expected: &[&str]) {
    assert_eq!(pages(body), expected, "{body}");
}

const TABLE: &str = "<table style='border-spacing:0;orphans:1;widows:1'>";
const HEADER: &str = "<thead><tr><td style='padding:0;line-height:20px'>H</td></tr></thead>";
const FOOTER: &str = "<tfoot><tr><td style='padding:0;line-height:20px'>F</td></tr></tfoot>";

#[test]
fn groups_that_do_not_repeat_leave_rows_to_break() {
    expect(
        &format!(
            "{TABLE}<thead><tr><td>H</td></tr></thead><tr><td>X</td></tr><tr><td>A<br>B</td></tr></table>"
        ),
        &["100: A@1,65 H@1,1 X@1,33", "100: B@1,0"],
    );
    expect(
        &format!(
            "<div style='height:50px'></div>{TABLE}<tfoot><tr><td>F</td></tr></tfoot><tr><td>A<br>B</td></tr></table>"
        ),
        &["100: A@1,51", "100: B@1,0 F@1,32"],
    );
}

#[test]
fn lines_moved_to_the_next_page_go_below_the_header_copy() {
    expect(
        &format!(
            "{TABLE}{HEADER}<tr><td>X</td></tr><tr><td>A<br>B</td></tr><tr><td>Z</td></tr></table>"
        ),
        &["100: A@1,53 H@0,-5 X@1,21", "100: B@1,20 H@0,-5 Z@1,52"],
    );
    expect(
        &format!("{TABLE}{HEADER}<tr><td>A<br>B<br>C<br>D<br>E</td></tr></table>"),
        &[
            "100: A@1,21 B@1,51 H@0,-5",
            "100: C@1,20 D@1,50 H@0,-5",
            "100: E@1,20 H@0,-5",
        ],
    );
    // A line that starts the next page right at its top goes below the
    // copy too.
    expect(
        &format!(
            "<div style='height:20px'></div>{TABLE}{HEADER}<tr><td style='padding:0'>A<br>B<br>C<br>D</td></tr></table>"
        ),
        &["100: A@0,40 B@0,70 H@0,15", "100: C@0,20 D@0,50 H@0,-5"],
    );
}

#[test]
fn the_page_a_row_leaves_ends_above_the_footer_copy() {
    expect(
        &format!(
            "{TABLE}{FOOTER}<tr><td>X</td></tr><tr><td>A<br>B</td></tr><tr><td>Z</td></tr></table>"
        ),
        &["100: A@1,33 F@0,75 X@1,1", "100: B@1,0 F@0,58 Z@1,32"],
    );
}

#[test]
fn the_last_row_leaves_room_for_the_footer_copy() {
    // The last row splits, and the page it leaves still ends with a copy.
    expect(
        &format!("{TABLE}{FOOTER}<tr><td>X</td></tr><tr><td>A<br>B</td></tr></table>"),
        &["100: A@1,33 F@0,75 X@1,1", "100: B@1,0 F@0,26"],
    );
    // A last row that cannot fit above the copy moves whole.
    expect(
        &format!(
            "{TABLE}{FOOTER}<tr><td>X</td></tr><tr><td>Y</td></tr><tr><td>Z</td></tr></table>"
        ),
        &["100: F@0,59 X@1,1 Y@1,33", "100: F@0,27 Z@1,1"],
    );
    expect(
        &format!(
            "{TABLE}{FOOTER}<tr><td>X</td></tr><tr><td>Y</td></tr><tr><td>A<br>B</td></tr></table>"
        ),
        &["100: F@0,59 X@1,1 Y@1,33", "100: A@1,1 B@1,31 F@0,57"],
    );
    // Lines that move in two cells of the last row grow it once.
    expect(
        &format!(
            "{TABLE}{FOOTER}<tr><td>X</td></tr><tr><td>A<br>B</td><td>C<br>D</td></tr></table>"
        ),
        &[
            "100: A@1,33 C@23,33 F@0,75 X@1,1",
            "100: B@1,0 D@23,0 F@0,26",
        ],
    );
    expect(
        &format!(
            "{TABLE}{FOOTER}<tr><td>X</td></tr><tr><td>A<br>B<br>C<br>D<br>E</td><td>P<br>Q<br>R<br>S<br>T</td></tr></table>"
        ),
        &[
            "100: A@1,33 F@0,75 P@23,33 X@1,1",
            "100: B@1,0 C@1,30 F@0,75 Q@23,0 R@23,30",
            "100: D@1,0 E@1,30 F@0,56 S@23,0 T@23,30",
        ],
    );
    // With a header too, every page the row crosses keeps both copies.
    expect(
        &format!(
            "{TABLE}{HEADER}{FOOTER}<tr><td>A<br>B<br>C<br>D</td></tr><tr><td>Z</td></tr></table>"
        ),
        &[
            "100: A@1,21 F@0,75 H@0,-5",
            "100: B@1,20 C@1,50 F@0,75 H@0,-5",
            "100: D@1,20 F@0,46 H@0,-5",
            "100: F@0,47 H@0,-5 Z@1,21",
        ],
    );
}

#[test]
fn a_tall_row_whose_first_line_does_not_fit_starts_the_next_page() {
    expect(
        &format!(
            "<div style='height:75px'></div>{TABLE}<tr><td>A<br>B<br>C<br>D</td></tr></table>"
        ),
        &["100: ", "100: A@1,1 B@1,31 C@1,61", "100: D@1,0"],
    );
    expect(
        &format!(
            "<table style='border-spacing:0'>{HEADER}<tr><td>X</td></tr><tr><td>A<br>B<br>C<br>D</td></tr></table>"
        ),
        &[
            "100: H@0,-5 X@1,21",
            "100: A@1,21 B@1,51 H@0,-5",
            "100: C@1,20 D@1,50 H@0,-5",
        ],
    );
    expect(
        &format!(
            "<div style='height:55px'></div>{TABLE}{HEADER}<tr><td>A<br>B<br>C<br>D</td></tr></table>"
        ),
        &[
            "100: ",
            "100: A@1,21 B@1,51 H@0,-5",
            "100: C@1,20 D@1,50 H@0,-5",
        ],
    );
}

#[test]
fn a_row_too_tall_for_the_room_below_the_header_copy_breaks_below_it() {
    expect(
        &format!(
            "<div style='height:30px'></div>{TABLE}{HEADER}<tr><td>X</td></tr><tr><td>A<br>B<br>C</td></tr><tr><td>Z</td></tr></table>"
        ),
        &[
            "100: H@0,25 X@1,51",
            "100: A@1,21 B@1,51 H@0,-5",
            "100: C@1,20 H@0,-5 Z@1,52",
        ],
    );
}

#[test]
fn lines_after_a_forced_break_in_a_cell_go_below_the_header_copy() {
    expect(
        &format!(
            "<style>td.f::before{{content:'A';display:block;break-after:page}}</style>{TABLE}{HEADER}<tr><td class=f>B</td></tr></table>"
        ),
        &["100: A@1,21 H@0,-5", "100: B@1,20 H@0,-5"],
    );
}

#[test]
fn lines_moved_inside_a_taller_cell_leave_the_footer_on_the_rows_last_page() {
    // The cell's height holds the moved line, so the row does not grow past
    // it and the footer stays on the second page.
    let pages = pages(&format!(
        "{TABLE}{FOOTER}<tr><td>X</td></tr><tr><td style='height:120px;padding:0;vertical-align:top'>A<br>B</td></tr></table>"
    ));
    assert_eq!(pages.len(), 2, "{pages:?}");
    assert_eq!(pages[0], "100: A@0,32 F@0,75 X@1,1");
    assert!(pages[1].starts_with("100: B@0,0 F@0,"), "{pages:?}");
}

/// A cell of a definite height whose content a forced break splits.
fn split_cell(height: &str) -> String {
    format!(
        "<tr><td style='padding:0;vertical-align:top;{height}'><div style='height:30px'>A</div>\
         <div style='height:30px;break-before:page'>B</div></td></tr>"
    )
}

#[test]
fn a_forced_break_in_a_cell_repeats_the_header_and_footer() {
    expect(
        &format!(
            "{TABLE}{HEADER}{}{FOOTER}</table><div>X</div>",
            split_cell("height:150px")
        ),
        &[
            "100: A@0,20 F@0,75 H@0,-5",
            "100: B@0,20 F@0,75 H@0,-5",
            "100: F@0,45 H@0,-5 X@0,70",
        ],
    );
    expect(
        &format!(
            "{TABLE}{HEADER}{}</table><div>X</div>",
            split_cell("height:150px")
        ),
        &["100: A@0,20 H@0,-5", "100: B@0,20 H@0,-5", "100: X@0,0"],
    );
}

#[test]
fn the_copies_count_toward_the_height_of_a_cell_split_by_a_forced_break() {
    // The row runs on past the page the break opens, with copies on each
    // page it crosses.
    expect(
        &format!(
            "{TABLE}{HEADER}{}{FOOTER}</table><div>X</div>",
            split_cell("height:250px")
        ),
        &[
            "100: A@0,20 F@0,75 H@0,-5",
            "100: B@0,20 F@0,75 H@0,-5",
            "100: F@0,75 H@0,-5",
            "100: F@0,75 H@0,-5",
            "100: F@0,25 H@0,-5 X@0,50",
        ],
    );
    expect(
        &format!(
            "{TABLE}{}{FOOTER}</table><div>X</div>",
            split_cell("height:250px")
        ),
        &[
            "100: A@0,0 F@0,75",
            "100: B@0,0 F@0,75",
            "100: F@0,75",
            "100: F@0,5 X@0,30",
        ],
    );
}

#[test]
fn rows_after_a_cell_split_by_a_forced_break_follow_its_last_page() {
    expect(
        &format!(
            "{TABLE}{HEADER}{}<tr><td style='padding:0'>Z</td></tr>{FOOTER}</table><div>X</div>",
            split_cell("height:150px")
        ),
        &[
            "100: A@0,20 F@0,75 H@0,-5",
            "100: B@0,20 F@0,75 H@0,-5",
            "100: F@0,75 H@0,-5 Z@0,50",
            "100: X@0,0",
        ],
    );
}

#[test]
fn a_footer_written_before_the_rows_repeats_around_a_forced_break_in_a_cell() {
    expect(
        &format!(
            "{TABLE}{FOOTER}<tr><td>Y</td></tr>{}</table><div>X</div>",
            split_cell("height:150px")
        ),
        &[
            "100: A@0,32 F@0,75 Y@1,1",
            "100: B@0,0 F@0,75",
            "100: F@0,17 X@0,42",
        ],
    );
}
