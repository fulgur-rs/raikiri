//! A table row beside a cell of an earlier row that spans it. When a cell of
//! the row begins with a box that no break goes through and that runs past
//! the page end, nothing of that cell goes before the break, so the row's own
//! cells start at the top of the next page and the row keeps its height there,
//! the share of the spanning cell's height included. The spanning cell goes
//! on from where it broke. Where each glyph falls is what Chromium prints for
//! the same documents on pages 200px wide and 100px tall, with Ahem glyphs of
//! 10px on lines of 20px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph, left edge and glyph top of every glyph of text in
/// `body`, laid out on the pages and with the font of the Chromium documents.
fn glyphs(body: &str) -> Vec<(u32, char, i32, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}} body{{margin:0;font:10px/20px Ahem}} table{{border-spacing:0}} td{{padding:0;vertical-align:top}}</style><body>{body}</body>"
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
            // Every Ahem glyph is 10px wide.
            for (offset, glyph) in run.text.chars().enumerate() {
                if !glyph.is_whitespace() {
                    glyphs.push((
                        index,
                        glyph,
                        (run.origin.0 + 10.0 * offset as f32).round() as i32,
                        (run.origin.1 - run.ascent).round() as i32,
                    ));
                }
            }
        }
    }
    glyphs.sort_unstable();
    glyphs
}

#[test]
fn the_row_starts_on_the_next_page_and_the_rows_after_it_follow() {
    // The row starts 29px above the page end beside the spanning cell; its
    // cell begins with a box that avoids a break inside it.
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:70px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Y', 10, 5),
            (2, 'R', 0, 5),
            (2, 'S', 10, 5),
            (2, 'X', 0, 25),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:70px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td><div style="height:70px;break-inside:avoid">Y</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Y', 10, 5),
            (2, 'X', 0, 5),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:120px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td><div style="height:70px;break-inside:avoid">Y</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Y', 10, 5),
            (2, 'X', 0, 33),
        ]
    );
}

#[test]
fn the_row_keeps_its_share_of_the_spanning_cell_on_the_next_page() {
    // The row is taller than its content by its share of the spanning cell's
    // height, and keeps that share and the padding of its cell.
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:100px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Y', 10, 5),
            (2, 'R', 0, 22),
            (2, 'S', 10, 22),
            (2, 'X', 0, 42),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:120px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td style="padding-top:20px"><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Y', 10, 25),
            (2, 'R', 0, 46),
            (2, 'S', 10, 46),
            (2, 'X', 0, 66),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td><td><div style="height:54px">W</div></td></tr><tr><td><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'R', 0, 84),
            (1, 'S', 10, 84),
            (1, 'Y', 10, 5),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn the_other_cells_of_the_row_go_along() {
    // A cell beside the one that breaks fits where it is, before or after it.
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td><td><div style="height:50px">W</div></td><td>V</td></tr><tr><td><div style="height:70px;break-inside:avoid">Y</div></td><td><div style="height:20px">Q</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'V', 20, 5),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Q', 20, 5),
            (1, 'Y', 10, 5),
            (2, 'R', 0, 5),
            (2, 'S', 10, 5),
            (2, 'X', 0, 25),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td><td><div style="height:50px">W</div></td><td>V</td></tr><tr><td><div style="height:20px">Q</div></td><td><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'V', 20, 5),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Q', 10, 5),
            (1, 'Y', 20, 5),
            (2, 'R', 0, 5),
            (2, 'S', 10, 5),
            (2, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_nested_or_unbreakable_first_box_starts_the_row_on_the_next_page() {
    // The first box sits in a block, or is an image.
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td><div><div style="height:70px;break-inside:avoid">Y</div></div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Y', 10, 5),
            (2, 'R', 0, 5),
            (2, 'S', 10, 5),
            (2, 'X', 0, 25),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td><img style="display:block;width:10px;height:70px"><div>Z</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Z', 10, 75),
            (2, 'R', 0, 5),
            (2, 'S', 10, 5),
            (2, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_row_inside_a_longer_span_starts_on_the_next_page() {
    // The spanning cell spans the row after it too.
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=3><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>Q</td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Q', 10, 75),
            (1, 'Y', 10, 5),
            (2, 'R', 0, 5),
            (2, 'S', 10, 5),
            (2, 'X', 0, 25),
        ]
    );
}

#[test]
fn content_before_the_box_keeps_the_row_on_its_page() {
    // Content before the box stays on the page, and the row grows only as
    // far as its content ends below it.
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td><div style="height:5px">P</div><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'P', 10, 61),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'R', 0, 75),
            (1, 'S', 10, 75),
            (1, 'Y', 10, 5),
            (2, 'X', 0, 5),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:120px">D</div></td><td><div style="height:40px">W</div></td></tr><tr><td><div style="height:5px">P</div><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'P', 10, 82),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Y', 10, 5),
            (2, 'R', 0, 25),
            (2, 'S', 10, 25),
            (2, 'X', 0, 45),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:70px">D</div></td><td><div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>Q</td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Q', 10, 75),
            (1, 'Y', 10, 5),
            (2, 'R', 0, 5),
            (2, 'S', 10, 5),
            (2, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_first_box_taller_than_the_page_starts_the_row_on_the_next_page_too() {
    // The box breaks inside itself there.
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td><td><div style="height:50px">W</div></td></tr><tr><td><div style="height:150px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Y', 10, 5),
            (2, 'R', 0, 55),
            (2, 'S', 10, 55),
            (2, 'X', 0, 75),
        ]
    );
}

#[test]
fn white_space_between_the_tags_does_not_keep_the_row_on_its_page() {
    assert_eq!(
        glyphs(
            r#"<table> <tr> <td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td> <td><div style="height:50px">W</div></td> </tr> <tr> <td> <div style="height:70px;break-inside:avoid">Y</div> </td> </tr> <tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'Y', 10, 5),
            (2, 'R', 0, 5),
            (2, 'S', 10, 5),
            (2, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_forced_break_before_the_row_still_opens_a_page_of_its_own() {
    assert_eq!(
        glyphs(
            r#"<table><tr><td rowspan=2><div style="height:30px">A</div><div style="height:70px">B</div><div style="height:40px">D</div></td><td><div style="height:50px">W</div></td></tr><tr style="break-before:page"><td><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (2, 'Y', 10, 5),
            (3, 'R', 0, 5),
            (3, 'S', 10, 5),
            (3, 'X', 0, 25),
        ]
    );
}
