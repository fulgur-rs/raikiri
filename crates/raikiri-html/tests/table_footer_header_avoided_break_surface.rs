//! A table with a footer or header group, followed by a box that it may not
//! break away from: the footer group, or the last row below a repeated
//! header, starts the next page with that box. Where each glyph falls is what
//! Chromium prints for the same documents on pages 200px wide and 100px
//! tall, with Ahem glyphs of 20px on lines of 20px.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph and glyph top of every glyph of text in `body`, laid out
/// on the pages and with the font of the Chromium documents. Every glyph
/// starts its line, at the left edge.
fn glyphs(body: &str) -> Vec<(u32, char, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}} body{{margin:0;font:20px/20px Ahem}} table{{border-spacing:0}} td{{padding:0}}</style><body>{body}</body>"
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
    let mut glyphs = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for glyph in run.text.chars().filter(|glyph| !glyph.is_whitespace()) {
                glyphs.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
            }
        }
    }
    glyphs.sort_unstable();
    glyphs
}

#[test]
fn a_footer_moves_with_a_box_that_avoids_the_break_before_it() {
    // The footer group is the table's last part: it starts the next page with the box, and the page before keeps no copy of it.
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'K', 30),
            (0, 'L', 50),
            (0, 'S', 0),
            (1, 'F', 0),
            (1, 'Z', 20)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table style="break-after:avoid"><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div>Z</div>"#
        ),
        [
            (0, 'K', 30),
            (0, 'L', 50),
            (0, 'S', 0),
            (1, 'F', 0),
            (1, 'Z', 20)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><table><tfoot><tr><td>F</td></tr></tfoot><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [(0, 'L', 50), (0, 'S', 0), (1, 'F', 0), (1, 'Z', 20)]
    );
}

#[test]
fn a_footer_of_several_rows_moves_whole() {
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><table><tfoot><tr><td>F</td></tr><tr><td>G</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'K', 10),
            (0, 'L', 30),
            (0, 'S', 0),
            (1, 'F', 0),
            (1, 'G', 20),
            (1, 'Z', 40)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><table><tfoot style="break-inside:avoid"><tr><td>F</td></tr><tr><td>G</td></tr></tfoot><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'L', 50),
            (0, 'S', 0),
            (1, 'F', 0),
            (1, 'G', 20),
            (1, 'Z', 40)
        ]
    );
}

#[test]
fn a_footer_tied_to_the_last_row_moves_with_it_and_repeats() {
    // A break that the footer may not take ties it to the last row: the row moves too, and the page before closes with a copy of the footer.
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><tfoot style="break-before:avoid"><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 50),
            (0, 'K', 30),
            (0, 'S', 0),
            (1, 'F', 20),
            (1, 'L', 0),
            (1, 'Z', 40)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr style="break-after:avoid"><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 50),
            (0, 'K', 30),
            (0, 'S', 0),
            (1, 'F', 20),
            (1, 'L', 0),
            (1, 'Z', 40)
        ]
    );
}

#[test]
fn a_table_with_a_footer_that_avoids_breaks_inside_it_moves_whole() {
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table style="break-inside:avoid"><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'S', 0),
            (1, 'F', 40),
            (1, 'K', 0),
            (1, 'L', 20),
            (1, 'Z', 60)
        ]
    );
}

#[test]
fn a_break_forced_after_the_footer_keeps_the_table_on_its_page() {
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><tfoot style="break-after:page"><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 70),
            (0, 'K', 30),
            (0, 'L', 50),
            (0, 'S', 0),
            (1, 'Z', 0)
        ]
    );
}

#[test]
fn a_caption_below_a_footer_is_the_last_part() {
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><caption style="caption-side:bottom">C</caption><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 70),
            (0, 'K', 30),
            (0, 'L', 50),
            (0, 'S', 0),
            (1, 'C', 0),
            (1, 'Z', 20)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><table><caption style="caption-side:bottom">C</caption><tfoot><tr><td>F</td></tr></tfoot><tr style="break-after:avoid"><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 50),
            (0, 'K', 10),
            (0, 'L', 30),
            (0, 'S', 0),
            (1, 'C', 0),
            (1, 'Z', 20)
        ]
    );
}

#[test]
fn a_caption_tied_to_the_footer_moves_with_it() {
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><table><caption style="caption-side:bottom;break-before:avoid">C</caption><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'K', 10),
            (0, 'L', 30),
            (0, 'S', 0),
            (1, 'C', 20),
            (1, 'F', 0),
            (1, 'Z', 40)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><table><caption style="caption-side:bottom">C</caption><tfoot style="break-after:avoid"><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'K', 10),
            (0, 'L', 30),
            (0, 'S', 0),
            (1, 'C', 20),
            (1, 'F', 0),
            (1, 'Z', 40)
        ]
    );
}

#[test]
fn a_table_with_a_footer_and_a_box_that_fit_stay() {
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><table><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 50),
            (0, 'K', 10),
            (0, 'L', 30),
            (0, 'S', 0),
            (0, 'Z', 70)
        ]
    );
}

#[test]
fn a_footer_inside_a_block_moves_with_the_box_after_it() {
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><div><table><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div></div>"#
        ),
        [
            (0, 'K', 30),
            (0, 'L', 50),
            (0, 'S', 0),
            (1, 'F', 0),
            (1, 'Z', 20)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div><div style="height:50px">S</div><table style="border-spacing:0"><tr><td>L</td></tr><tfoot><tr><td>F</td></tr></tfoot></table><div style="break-before:avoid">Z</div></div>"#
        ),
        [(0, 'L', 50), (0, 'S', 0), (1, 'F', 0), (1, 'Z', 20)]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><div><table><tfoot><tr><td>F</td></tr></tfoot><tr><td>L</td></tr></table></div><div style="break-before:avoid">Z</div>"#
        ),
        [(0, 'L', 50), (0, 'S', 0), (1, 'F', 0), (1, 'Z', 20)]
    );
}

#[test]
fn a_footer_that_moves_opens_its_page_below_a_copy_of_the_header() {
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><table><thead><tr><td>H</td></tr></thead><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'H', 10),
            (0, 'K', 30),
            (0, 'L', 50),
            (0, 'S', 0),
            (1, 'F', 20),
            (1, 'H', 0),
            (1, 'Z', 40)
        ]
    );
}

#[test]
fn the_last_row_of_a_table_with_a_header_moves_below_a_copy_of_it() {
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><thead><tr><td>H</td></tr></thead><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'H', 30),
            (0, 'K', 50),
            (0, 'S', 0),
            (1, 'H', 0),
            (1, 'L', 20),
            (1, 'Z', 40)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><thead><tr><td>H</td></tr></thead><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'H', 30),
            (0, 'K', 50),
            (0, 'S', 0),
            (1, 'H', 0),
            (1, 'L', 20),
            (1, 'Z', 40)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><table><thead><tr><td>H</td></tr></thead><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [(0, 'S', 0), (1, 'H', 0), (1, 'L', 20), (1, 'Z', 40)]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><table><thead><tr><td>H</td></tr></thead><tr><td>J</td></tr><tr><td>K</td></tr><tr style="break-before:avoid"><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'H', 10),
            (0, 'J', 30),
            (0, 'S', 0),
            (1, 'H', 0),
            (1, 'K', 20),
            (1, 'L', 40),
            (1, 'Z', 60)
        ]
    );
}

#[test]
fn the_last_row_of_a_table_with_a_header_inside_a_block_moves_below_a_copy_of_it() {
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><div><table><thead><tr><td>H</td></tr></thead><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div></div>"#
        ),
        [
            (0, 'H', 30),
            (0, 'K', 50),
            (0, 'S', 0),
            (1, 'H', 0),
            (1, 'L', 20),
            (1, 'Z', 40)
        ]
    );
}

#[test]
fn a_header_that_does_not_repeat_stays_with_the_first_rows() {
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><thead style="break-inside:auto"><tr><td>H</td></tr></thead><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'H', 30),
            (0, 'K', 50),
            (0, 'S', 0),
            (1, 'L', 0),
            (1, 'Z', 20)
        ]
    );
}

#[test]
fn a_box_tied_to_a_table_with_a_header_moves_with_its_first_row() {
    // The page holds the header and the first row below it with the box before the table.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><div style="break-after:avoid">A</div><table><thead><tr><td>H</td></tr></thead><tr><td>K</td></tr><tr><td>L</td></tr></table>"#
        ),
        [
            (0, 'S', 0),
            (1, 'A', 0),
            (1, 'H', 20),
            (1, 'K', 40),
            (1, 'L', 60)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:70px">S</div><div style="break-after:avoid">A</div><table><thead><tr><td>H</td></tr></thead><tr><td>K</td></tr><tr><td>L</td></tr></table>"#
        ),
        [
            (0, 'S', 0),
            (1, 'A', 0),
            (1, 'H', 20),
            (1, 'K', 40),
            (1, 'L', 60)
        ]
    );
}

#[test]
fn a_footer_that_does_not_fit_below_the_last_row_takes_the_row_along() {
    // The footer would cross the page end below the last row: the page breaks
    // before the rows that the last one may not break from instead, closes with
    // a copy of the footer, and the next page repeats the header, if any.
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><thead><tr><td>H</td></tr></thead><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 70),
            (0, 'H', 30),
            (0, 'K', 50),
            (0, 'S', 0),
            (1, 'F', 40),
            (1, 'H', 0),
            (1, 'L', 20),
            (1, 'Z', 60)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><thead style="break-inside:avoid"><tr><td>H</td></tr></thead><tfoot><tr><td>F</td></tr></tfoot><tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 70),
            (0, 'H', 30),
            (0, 'K', 50),
            (0, 'S', 0),
            (1, 'F', 40),
            (1, 'H', 0),
            (1, 'L', 20),
            (1, 'Z', 60)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><tfoot><tr><td>F</td></tr></tfoot><tbody><tr><td>K</td></tr></tbody><tbody style="break-inside:avoid"><tr><td>L</td></tr><tr><td>M</td></tr></tbody></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 50),
            (0, 'K', 30),
            (0, 'S', 0),
            (1, 'F', 40),
            (1, 'L', 0),
            (1, 'M', 20),
            (1, 'Z', 60)
        ]
    );
}

#[test]
fn a_break_forced_between_the_footer_rows_leaves_the_rows_before_it() {
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><tfoot><tr><td>F</td></tr><tr style="break-before:page"><td>G</td></tr></tfoot><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 50),
            (0, 'L', 30),
            (0, 'S', 0),
            (1, 'G', 0),
            (1, 'Z', 20)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><tfoot><tr style="break-after:page"><td>F</td></tr><tr><td>G</td></tr></tfoot><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 50),
            (0, 'L', 30),
            (0, 'S', 0),
            (1, 'G', 0),
            (1, 'Z', 20)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><table><tfoot><tr><td>F</td></tr><tr><td>G</td></tr><tr style="break-before:page"><td>H</td></tr></tfoot><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'F', 30),
            (0, 'G', 50),
            (0, 'L', 10),
            (0, 'S', 0),
            (1, 'H', 0),
            (1, 'Z', 20)
        ]
    );
}
