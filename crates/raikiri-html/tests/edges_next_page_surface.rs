//! The padding and border that close a box and do not fit under its last line
//! or box on the page that content ends on: they start the next page, even
//! when they are taller than a page, and split from there. Boxes that nest each
//! judge their own edges, the innermost first. A block with nothing in it has
//! its edges pushed the same way, the edges a clip hides stay where they are,
//! and an image keeps its own. Where each line of text falls is what Chromium
//! prints for the same documents.
//!
//! Each row is a document and the page and top of every line of text in it (a
//! marker `Z` in a block after the box shows where the box ends), as Chromium
//! prints it on pages 100px tall and 100px wide. The first table has Ahem
//! lines of 20px with a line height of 30px, the second of 10px with a line
//! height of 10px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page and top of each line of text of `body` laid out in `font`, a
/// `font` shorthand without the family. `baseline` is how far below the top of
/// a line the text's baseline sits: Ahem's ascent is 0.8em, and its glyphs sit
/// in the middle of a line taller than the font size.
fn line_tops(body: &str, font: &str, baseline: f32) -> Vec<(usize, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}         body{{margin:0;font:{font} Ahem}}</style><body>{body}</body>"
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
    for index in 0..30 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            // The words of a line are runs of their own in some paragraphs.
            tops.push((index as usize, (run.origin.1 - baseline).round() as i32));
        }
    }
    tops.sort_unstable();
    tops.dedup();
    tops
}

type Row = (&'static str, &'static [(usize, i32)]);

fn check(rows: &[Row], font: &str, baseline: f32) {
    for &(body, expected) in rows {
        assert_eq!(line_tops(body, font, baseline), expected, "{body}");
    }
}

#[test]
fn edges_that_do_not_fit_under_the_content_start_the_next_page_with_30px_lines() {
    check(ROWS_30PX_LINES, "20px/30px", 21.0);
}

#[test]
fn edges_that_do_not_fit_under_the_content_start_the_next_page_with_10px_lines() {
    check(ROWS_10PX_LINES, "10px/10px", 8.0);
}

const ROWS_30PX_LINES: &[Row] = &[
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 50), (1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(0, 50), (1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 70)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 70)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (2, 45)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (2, 45)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (2, 70)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (2, 70)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(4, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(1, 55)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(4, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (4, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:300px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (5, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (5, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:300px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (6, 0)],
    ),
    (
        r#"<div style="width:20px;border-bottom:95px solid"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:47px;border-bottom:48px solid"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;border-bottom:130px solid"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:65px;border-bottom:65px solid"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:20px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:20px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0), (2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:60px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:60px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-top:20px;padding-bottom:60px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 20), (1, 60)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="padding-bottom:20px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 60)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="margin-bottom:10px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0), (3, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:95px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:95px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-top:20px;padding-bottom:95px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 20), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="padding-bottom:20px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="margin-bottom:10px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (2, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0), (3, 30)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:130px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:130px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-top:20px;padding-bottom:130px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 20), (2, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="padding-bottom:20px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (2, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="margin-bottom:10px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (3, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0), (4, 50)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:250px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (4, 50)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:250px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(4, 50)],
    ),
    (
        r#"<div style="width:20px;padding-top:20px;padding-bottom:250px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 20), (3, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="padding-bottom:20px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (3, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><div style="margin-bottom:10px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (4, 50)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:40px"><div style="padding-bottom:40px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:60px"><div style="padding-bottom:20px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:20px"><div style="padding-bottom:60px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:80px"><div style="padding-bottom:30px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:10px"><div style="padding-bottom:95px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 10)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:95px"><div style="padding-bottom:10px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:40px"><div style="padding-bottom:40px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:60px"><div style="padding-bottom:20px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:20px"><div style="padding-bottom:60px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:80px"><div style="padding-bottom:30px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:10px"><div style="padding-bottom:95px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 10)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:95px"><div style="padding-bottom:10px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:40px"><div style="padding-bottom:40px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:60px"><div style="padding-bottom:20px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:20px"><div style="padding-bottom:60px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:80px"><div style="padding-bottom:30px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:10px"><div style="padding-bottom:95px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 10)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:95px"><div style="padding-bottom:10px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
];

const ROWS_10PX_LINES: &[Row] = &[
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"><div style="height:40px">A</div><div style="height:30px">B</div><div style="height:20px">C</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 40), (1, 0), (1, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:90px"><div style="height:40px">A</div><div style="height:30px">B</div><div style="height:20px">C</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 40), (1, 0), (2, 90)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:15px"><div style="height:10px">A</div><div style="height:90px">B</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (2, 15)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:100px"><div style="height:10px">A</div><div style="height:90px">B</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (3, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:250px"><div style="height:10px">A</div><div style="height:90px">B</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (4, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"><div style="height:80px">A</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:50px">S</div><div style="padding-bottom:95px"><div style="width:30px">aaa aaa</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 50), (1, 0), (3, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:50px">S</div><div style="padding-bottom:250px"><div style="width:30px">aaa aaa</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 50), (1, 0), (4, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:90px"><div style="height:10px">S</div><div style="width:30px">aaa aaa aaa aaa aaa aaa</div></div><div style="height:10px">Z</div>"#,
        &[
            (0, 0),
            (0, 10),
            (0, 20),
            (0, 30),
            (0, 40),
            (1, 0),
            (1, 10),
            (2, 90),
        ],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:90px"><div style="height:60px">S</div><div style="width:30px;padding-bottom:5px">aaa</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (2, 90)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="border-bottom:30px solid"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"><div style="height:0"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"> </div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:20px"><div style="padding-bottom:30px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><p style="margin:0;padding-bottom:30px"></p><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="height:40px;overflow:hidden"><div style="padding-bottom:50px"><div style="height:20px">A</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 60), (1, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><img style="display:block;width:20px;height:90px;padding-bottom:30px"><div style="height:10px">Z</div>"#,
        &[(1, 20)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:20px">S</div><img style="display:block;width:20px;height:90px;padding-bottom:30px"><div style="height:10px">Z</div>"#,
        &[(0, 0), (2, 20)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"><div style="float:left;width:10px;height:10px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"><div style="position:absolute;width:10px;height:10px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:100px">S</div><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:250px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (3, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:70px">S</div><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="display:flow-root;padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="padding-bottom:20px"><div style="height:30px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="height:10px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (1, 56)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"><div style="height:60px">A</div><div style="height:50px"><div style="height:60px">B</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 60), (1, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"><div style="height:60px">A</div><div style="height:50px"><div style="height:20px">B</div><div style="padding-bottom:30px"><div style="height:10px">C</div></div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 60), (1, 0), (1, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="padding-bottom:70px"><div style="height:30px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (2, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="padding-bottom:80px"><div style="height:30px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (2, 80)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="padding-bottom:120px"><div style="height:30px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (3, 20)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:90px">S</div><div style="display:inline-block;width:40px;height:26px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="padding-bottom:30px;break-before:page"><div style="height:10px">D</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 18), (2, 0), (2, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:90px">S</div><div style="display:inline-block;vertical-align:top;width:40px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="padding-bottom:30px;break-before:page"><div style="height:10px">D</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (2, 0), (2, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:90px">S</div><div style="display:inline-block;width:40px;height:26px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="padding-bottom:95px;break-before:page"><div style="height:10px">D</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 18), (2, 0), (4, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:90px">S</div><div style="display:inline-block;vertical-align:top;width:40px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="padding-bottom:95px;break-before:page"><div style="height:10px">D</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (2, 0), (4, 0)],
    ),
];
