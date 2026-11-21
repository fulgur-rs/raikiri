//! A box at the top of a row that the page cannot break before (a flex
//! container whose item margin runs past the page end, a box that avoids a
//! break inside it, or one no break goes through) moves the whole row, and
//! with it the flex row, the grid of several columns or the single column
//! that holds it, to the next page. Expected positions are Chromium's.

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

/// The page, glyph, left and top of every glyph in `body`, sorted. The top
/// is that of the glyph's line, which has 5px of half-leading above the
/// Ahem glyph.
fn glyphs(body: &str) -> Vec<(u32, char, i32, i32)> {
    let result = lay_out(body);
    let mut glyphs = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let mut x = run.origin.0;
            for glyph in run.text.chars() {
                if !glyph.is_whitespace() {
                    let top = (run.origin.1 - run.ascent - 5.0).round() as i32;
                    glyphs.push((index, glyph, x.round() as i32, top));
                }
                x += 20.0;
            }
        }
    }
    glyphs.sort();
    glyphs
}

const LEAD: &str = r#"<div style="height:60px">A</div>"#;
const MARGINS: &str = r#"<div style="display:flex;flex-direction:column"><div style="height:35px;margin-bottom:6px">T</div></div>"#;
const AVOID: &str = r#"<div style="height:50px;break-inside:avoid">T</div>"#;
const MONOLITHIC: &str = r#"<div style="contain:size;height:50px;overflow:hidden">T</div>"#;

fn grid(first: &str, second: &str) -> String {
    format!(
        r#"{LEAD}<div style="display:grid;grid-template-columns:50px 50px"><div>{first}</div><div>{second}</div></div><div>Z</div>"#
    )
}

fn flex_row(first: &str) -> String {
    format!(
        r#"{LEAD}<div style="display:flex"><div style="width:50px">{first}</div><div>B</div></div><div>Z</div>"#
    )
}

fn single_column(content: &str) -> String {
    format!(r#"{LEAD}<div style="columns:1">{content}</div><div>Z</div>"#)
}

fn moved_row(z: i32) -> Vec<(u32, char, i32, i32)> {
    vec![
        (0, 'A', 0, 0),
        (1, 'B', 50, 0),
        (1, 'T', 0, 0),
        (1, 'Z', 0, z),
    ]
}

#[test]
fn flex_margins_move_the_grid_row() {
    assert_eq!(glyphs(&grid(MARGINS, "B")), moved_row(41));
}

#[test]
fn flex_item_margins_move_the_grid_row() {
    let body = format!(
        r#"{LEAD}<div style="display:grid;grid-template-columns:50px 50px"><div style="display:flex;flex-direction:column"><div style="height:35px;margin-bottom:6px">T</div></div><div>B</div></div><div>Z</div>"#
    );
    assert_eq!(glyphs(&body), moved_row(41));
}

#[test]
fn flex_margins_move_the_flex_row() {
    assert_eq!(glyphs(&flex_row(MARGINS)), moved_row(41));
}

#[test]
fn flex_item_margins_move_the_flex_row() {
    let body = format!(
        r#"{LEAD}<div style="display:flex"><div style="width:50px;display:flex;flex-direction:column"><div style="height:35px;margin-bottom:6px">T</div></div><div>B</div></div><div>Z</div>"#
    );
    assert_eq!(glyphs(&body), moved_row(41));
}

#[test]
fn flex_margins_move_a_single_column() {
    assert_eq!(
        glyphs(&single_column(MARGINS)),
        vec![(0, 'A', 0, 0), (1, 'T', 0, 0), (1, 'Z', 0, 41)]
    );
}

#[test]
fn avoided_break_moves_the_grid_row() {
    assert_eq!(glyphs(&grid(AVOID, "B")), moved_row(50));
    let deep = format!("<div><div>{AVOID}</div></div>");
    assert_eq!(glyphs(&grid(&deep, "B")), moved_row(50));
}

#[test]
fn avoided_break_in_the_second_item_moves_the_grid_row() {
    assert_eq!(
        glyphs(&grid("B", AVOID)),
        vec![
            (0, 'A', 0, 0),
            (1, 'B', 0, 0),
            (1, 'T', 50, 0),
            (1, 'Z', 0, 50)
        ]
    );
}

#[test]
fn tall_avoided_break_still_moves_the_grid_row() {
    let tall = r#"<div style="height:150px;break-inside:avoid">T</div>"#;
    assert_eq!(
        glyphs(&grid(tall, "B")),
        vec![
            (0, 'A', 0, 0),
            (1, 'B', 50, 0),
            (1, 'T', 0, 0),
            (2, 'Z', 0, 50)
        ]
    );
}

#[test]
fn monolithic_box_moves_the_row_or_column() {
    assert_eq!(glyphs(&grid(MONOLITHIC, "B")), moved_row(50));
    assert_eq!(glyphs(&flex_row(MONOLITHIC)), moved_row(50));
    assert_eq!(
        glyphs(&single_column(MONOLITHIC)),
        vec![(0, 'A', 0, 0), (1, 'T', 0, 0), (1, 'Z', 0, 50)]
    );
}

#[test]
fn avoided_break_splits_in_a_single_column() {
    assert_eq!(
        glyphs(&single_column(AVOID)),
        vec![(0, 'A', 0, 0), (0, 'T', 0, 60), (1, 'Z', 0, 10)]
    );
}
