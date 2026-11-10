//! The lines of a paragraph in an absolutely positioned box around the end of
//! a page. Lines inside the only item of a flex or grid container break across
//! pages as lines in a block do. A paragraph that begins the content of the box
//! keeps a first line that fits on the page the box starts on, whatever its
//! `orphans`: a break before it would fall before the box itself. Where each
//! line falls is what Chromium prints for the same documents on pages 100px
//! tall, with Ahem lines of 20px and a line height of 30px.

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
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let Some(glyph) = run.text.trim().chars().next() else {
                continue;
            };
            tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
        }
    }
    tops.sort_unstable();
    tops.dedup();
    tops
}

/// A document and the page, glyph and top of each of its lines.
type Row = (&'static str, &'static [(u32, char, i32)]);

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}

#[test]
fn the_lines_of_the_only_item_break_across_pages() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:20px;display:grid'><div style='padding-top:15px'><p style='margin:0;orphans:1;widows:1'>X X X</p></div></div>",
            &[(1, 'X', 70), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px;display:flex;flex-direction:column'><div><p style='margin:0;orphans:1;widows:1'>X X X</p></div></div>",
            &[(1, 'X', 45), (1, 'X', 75), (2, 'X', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px;display:flex'><div><p style='margin:0;orphans:1;widows:1'>X X X</p></div></div>",
            &[(1, 'X', 45), (1, 'X', 75), (2, 'X', 5)],
        ),
    ]);
}

#[test]
fn a_paragraph_beginning_the_box_keeps_a_first_line_that_fits() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:20px;display:grid'><div style='padding-top:15px'><p style='margin:0;orphans:3;widows:1'>X X X</p></div></div>",
            &[(1, 'X', 70), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px'><div><p style='margin:0;orphans:3;widows:1'>X X X X</p></div></div>",
            &[(1, 'X', 45), (1, 'X', 75), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px'><div style='padding-top:15px'><p style='margin:0;orphans:3;widows:1'>X X X X</p></div></div>",
            &[(1, 'X', 60), (2, 'X', 5), (2, 'X', 35), (2, 'X', 65)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px'><div><p style='margin:0;orphans:3;widows:1;break-inside:avoid'>X X X</p></div></div>",
            &[(1, 'X', 45), (1, 'X', 75), (2, 'X', 5)],
        ),
    ]);
}

#[test]
fn a_paragraph_after_other_content_keeps_its_orphans() {
    assert_rows(&[
        // A first line that does not fit the page still starts the next one.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:60px;width:20px'><div style='padding-top:15px'><p style='margin:0;orphans:3;widows:1'>X X X X</p></div></div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'X', 5)],
        ),
        // Content before the paragraph in the box: the break before the paragraph
        // keeps its orphans.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:30px;width:20px'><div style='height:30px'>Q</div><p style='margin:0;orphans:3;widows:1'>X X X X</p></div>",
            &[
                (0, 'Q', 35),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'X', 5),
            ],
        ),
        // Content inside an element that generates no box counts as well.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:30px;width:20px'><div style='display:contents'><div style='height:30px'>Q</div></div><p style='margin:0;orphans:3;widows:1'>X X X X</p></div>",
            &[
                (0, 'Q', 35),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'X', 5),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:120px;width:20px'><div style='display:contents'><div style='margin-bottom:20px'></div></div><p style='margin:0;orphans:3;widows:1'>X X X X</p></div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65), (3, 'X', 5)],
        ),
        // So does an empty box whose margin puts the paragraph lower.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:120px;width:20px'><div style='margin-bottom:20px'></div><p style='margin:0;orphans:3;widows:1'>X X X X</p></div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65), (3, 'X', 5)],
        ),
        // So do the padding and border of a grid above its item.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px;display:grid;padding-top:10px'><div><p style='margin:0;orphans:3;widows:1'>X X X</p></div></div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65)],
        ),
    ]);
}
