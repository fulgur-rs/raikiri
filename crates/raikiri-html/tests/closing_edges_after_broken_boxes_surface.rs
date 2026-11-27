//! When the bottom padding and border that close the blocks around a box do
//! not fit its page, Chromium breaks before the box only inside the blocks
//! that begin on that page. A table or block that goes on from an earlier
//! page keeps its last row, footer group or child there, and the edges of the
//! blocks around it break away on their own. A table that begins on the page
//! still moves its footer group on with them.
//!
//! The positions are the ones Chromium prints for the same documents
//! (`page.pdf`, 100px square pages). Each page lists the 20px Ahem glyphs on
//! it as `glyph@x,top`, with the top of a 30px line; the 20px lines of the
//! tables read 5px above their top. Chromium paints the footer copies too,
//! although its PDF text lists only the first.

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

const TABLE: &str = "<table style='width:100%;line-height:20px;border-spacing:0'>";

fn rows(glyphs: &str) -> String {
    glyphs
        .chars()
        .map(|glyph| format!("<tr><td style='padding:0'>{glyph}</td></tr>"))
        .collect()
}

fn footer() -> String {
    format!("<tfoot>{}</tfoot>", rows("F"))
}

/// A block with 70px of bottom padding, which no page holds after the
/// content below, around a spacer of `lead` and `content`.
fn padded(lead: u32, content: &str) -> String {
    format!("<div style='padding-bottom:70px'><div style='height:{lead}px'></div>{content}</div>Z")
}

#[test]
fn the_footer_stays_with_the_last_row_of_a_table_from_an_earlier_page() {
    // The last row moves on to leave room for the footer's copy.
    expect(
        &padded(60, &format!("{TABLE}{}{}</table>", rows("AB"), footer())),
        &["100: A@0,55 F@0,75", "100: B@0,-5 F@0,15", "100: Z@0,70"],
    );
    // The rows run on to the next page.
    expect(
        &padded(30, &format!("{TABLE}{}{}</table>", rows("ABCDE"), footer())),
        &[
            "100: A@0,25 B@0,45 F@0,65",
            "100: C@0,-5 D@0,15 E@0,35 F@0,55",
            "100: Z@0,70",
        ],
    );
    // A bottom border closes the block instead.
    expect(
        &format!(
            "<div style='border-bottom:70px solid'><div style='height:60px'></div>\
             {TABLE}{}{}</table></div>Z",
            rows("AB"),
            footer()
        ),
        &["100: A@0,55 F@0,75", "100: B@0,-5 F@0,15", "100: Z@0,70"],
    );
}

#[test]
fn the_footer_of_a_table_that_begins_on_the_page_moves_with_the_edges() {
    expect(
        &padded(20, &format!("{TABLE}{}{}</table>", rows("AB"), footer())),
        &["100: A@0,15 B@0,35", "100: F@0,-5", "100: Z@0,0"],
    );
    // The table moves to the next page with its first row, its footer
    // written first.
    expect(
        &padded(85, &format!("{TABLE}{}{}</table>", footer(), rows("AB"))),
        &["100: ", "100: A@0,-5 B@0,15", "100: F@0,-5", "100: Z@0,0"],
    );
    // Spaces between the table's tags and a column group do not hold it on
    // the page it left.
    expect(
        &padded(
            85,
            &format!(
                "{TABLE} <colgroup><col></colgroup> {} <tbody>{}</tbody> </table>",
                footer(),
                rows("AB")
            ),
        ),
        &["100: ", "100: A@0,-5 B@0,15", "100: F@0,-5", "100: Z@0,0"],
    );
}

#[test]
fn the_last_row_stays_on_the_page_a_table_goes_on_to() {
    expect(
        &padded(30, &format!("{TABLE}{}</table>", rows("ABCDE"))),
        &[
            "100: A@0,25 B@0,45 C@0,65",
            "100: D@0,-5 E@0,15",
            "100: Z@0,70",
        ],
    );
    // Below the copy of the table's header.
    expect(
        &padded(
            30,
            &format!(
                "{TABLE}<thead>{}</thead>{}</table>",
                rows("H"),
                rows("ABCD")
            ),
        ),
        &[
            "100: A@0,45 B@0,65 H@0,25",
            "100: C@0,15 D@0,35 H@0,-5",
            "100: Z@0,70",
        ],
    );
}

#[test]
fn the_last_block_stays_in_a_block_from_an_earlier_page() {
    let blocks = |glyphs: &str| -> String {
        glyphs
            .chars()
            .map(|glyph| format!("<div style='height:20px;line-height:20px'>{glyph}</div>"))
            .collect()
    };
    expect(
        &padded(30, &format!("<div>{}</div>", blocks("ABCDE"))),
        &[
            "100: A@0,25 B@0,45 C@0,65",
            "100: D@0,-5 E@0,15",
            "100: Z@0,70",
        ],
    );
    // A block that the padding itself closes moves its last child on.
    expect(
        &padded(30, &blocks("ABCDE")),
        &[
            "100: A@0,25 B@0,45 C@0,65",
            "100: D@0,-5",
            "100: E@0,-5",
            "100: Z@0,0",
        ],
    );
}

#[test]
fn the_end_of_a_tied_run_stays_in_a_block_from_an_earlier_page() {
    expect(
        &padded(
            30,
            "<div><div style='height:20px;line-height:20px'>A</div>\
             <div style='height:20px;line-height:20px'>B</div>\
             <div style='height:20px;line-height:20px'>C</div>\
             <div style='height:20px;line-height:20px'>D</div>\
             <div style='height:20px;line-height:20px;break-before:avoid'>E</div></div>",
        ),
        &[
            "100: A@0,25 B@0,45 C@0,65",
            "100: D@0,-5 E@0,15",
            "100: Z@0,70",
        ],
    );
}

#[test]
fn the_last_lines_stay_in_a_block_from_an_earlier_page() {
    expect(
        &padded(
            30,
            "<div><p style='margin:0;line-height:20px'>A<br>B<br>C</p>\
             <p style='margin:0;line-height:20px'>D<br>E</p></div>",
        ),
        &[
            "100: A@0,25 B@0,45 C@0,65",
            "100: D@0,-5 E@0,15",
            "100: Z@0,70",
        ],
    );
}

#[test]
fn a_relative_offset_does_not_change_where_a_block_begins() {
    // The inner block begins at the top of the second page whatever its
    // offset paints it at, so the outer block's padding still pulls its last
    // child on to the third page.
    for top in [-10, 0, 10] {
        let body = format!(
            "<div style='padding-bottom:30px'><div style='height:100px'>A</div>\
             <div style='position:relative;top:{top}px;padding-bottom:20px'>\
             <div style='height:60px'>B</div><div style='height:20px'>X</div></div></div>\
             <div>Z</div>"
        );
        let b = format!("100: B@0,{top}");
        let x = format!("100: X@0,{top} Z@0,70");
        expect(&body, &["100: A@0,0", &b, &x]);
    }
}
