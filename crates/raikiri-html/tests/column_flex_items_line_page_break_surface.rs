//! The items of a column flex container in the normal flow break across a
//! page end as blocks do: the line that crosses the page end starts the next
//! page, and only an item that is a single line no text places moves whole.
//! In a column flex container that wraps, an item that grows or moves down
//! takes along only the items after it in its own column; the other columns
//! keep their places and break between their own lines. Where each line
//! falls is what Chromium prints for the same documents on pages 100px tall,
//! with Ahem lines of 20px and a line height of 30px.

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

/// A document body and the page, glyph and top of each line.
type Row = (&'static str, &'static [(u32, char, i32)]);

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}

#[test]
fn column_flex_items_break_between_their_lines() {
    // A paragraph item, an item that holds blocks, gaps, margins and borders,
    // and fixed-height or scrolling items with or without content: each
    // breaks where a block would. Orphans that the first page cannot hold
    // move the item, and so do the padding or border that close an item of
    // one line, which stay with that line.
    assert_rows(&[
        (
            "<div style='height:150px'></div><div style='width:20px;display:grid'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0'>Z</p></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 65)],
        ),
        (
            "<div style='height:150px'></div><div style='width:20px;display:flex;flex-direction:column'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0'>Z</p></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 65)],
        ),
        (
            "<div style='height:150px'></div><div style='width:20px;display:flex;flex-direction:column'><div style='margin:0'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0'>Z</p></div><p style='margin:0'>Q</p></div>",
            &[
                (1, 'X', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
                (3, 'Q', 5),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:20px;display:flex;flex-direction:column'><p style='margin:0;orphans:2;widows:2'>X X X</p><p style='margin:0'>Z</p></div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65), (3, 'Z', 5)],
        ),
        (
            "<div style='height:150px'></div><div style='width:20px;display:flex;flex-direction:column;gap:10px'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0'>Z</p></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 75)],
        ),
        (
            "<div style='height:150px'></div><div style='width:20px;display:flex;flex-direction:column'><p style='margin:10px 0;orphans:1;widows:1'>X X X</p><p style='margin:0'>Z</p></div>",
            &[(1, 'X', 65), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 75)],
        ),
        (
            "<div style='height:150px'></div><div style='width:20px;display:flex;flex-direction:column'><p style='margin:0;orphans:1;widows:1;border-bottom:10px solid'>X X X</p><p style='margin:0'>Z</p></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 75)],
        ),
        (
            "<div style='height:150px'></div><div style='width:20px;display:flex;flex-direction:column;justify-content:center;height:150px'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0'>Z</p></div>",
            &[(1, 'X', 70), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='display:flex;flex-direction:column'><div style='height:60px'>A</div><p style='margin:0'>Z</p></div>",
            &[(0, 'A', 55), (1, 'Z', 15)],
        ),
        (
            "<div style='height:80px'></div><div style='display:flex;flex-direction:column'><div style='height:60px'></div><p style='margin:0'>Z</p></div>",
            &[(1, 'Z', 45)],
        ),
        (
            "<div style='height:50px'></div><div style='display:flex;flex-direction:column'><div><p style='margin:0'>A</p><p style='margin:0'>B</p></div><p style='margin:0'>Z</p></div>",
            &[(0, 'A', 55), (1, 'B', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:50px'></div><div style='display:flex;flex-direction:column'><div style='height:60px;overflow:hidden'>A</div><p style='margin:0'>Z</p></div>",
            &[(0, 'A', 55), (1, 'Z', 15)],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 65)],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='display:flex;flex-direction:column'><div style='padding-bottom:40px;orphans:1;widows:1'>A A</div><p style='margin:0'>Z</p></div>",
            &[(1, 'A', 5), (1, 'Z', 75)],
        ),
        (
            "<div style='height:50px'></div><div style='display:flex;flex-direction:column'><div style='height:70px'>A</div><p style='margin:0'>Z</p></div>",
            &[(0, 'A', 55), (1, 'Z', 25)],
        ),
        (
            "<div style='height:50px'></div><div style='display:flex;flex-direction:column'><div style='border-bottom:40px solid'>A</div><p style='margin:0'>Z</p></div>",
            &[(1, 'A', 5), (1, 'Z', 75)],
        ),
        (
            "<div style='height:50px'></div><div style='display:flex;flex-direction:column'><div style='height:60px;overflow:auto'>A</div><p style='margin:0'>Z</p></div>",
            &[(0, 'A', 55), (1, 'Z', 15)],
        ),
    ]);
}

#[test]
fn wrapped_column_flex_items_move_only_their_own_column() {
    // The item after a grown or moved one in its column follows it; an item
    // in the next column keeps its place and breaks between its own lines,
    // however the columns are ordered or aligned.
    assert_rows(&[
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y</p></div>",
            &[
                (1, 'W', 55),
                (1, 'X', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 65)],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:90px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(1, 'X', 55), (1, 'Z', 55), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>Q Q Q Q</p><p style='margin:0;orphans:1;widows:1;width:20px'>W</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y</p></div>",
            &[
                (1, 'Q', 55),
                (1, 'W', 55),
                (2, 'Q', 5),
                (2, 'Q', 35),
                (2, 'Q', 65),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>Q</p><p style='margin:0;orphans:1;widows:1;width:20px'>R</p><p style='margin:0;orphans:1;widows:1;width:20px'>S</p><p style='margin:0;orphans:1;widows:1;width:20px'>T</p><p style='margin:0;orphans:1;widows:1;width:20px'>W</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y</p></div>",
            &[
                (1, 'Q', 55),
                (1, 'W', 55),
                (2, 'R', 5),
                (2, 'S', 35),
                (2, 'T', 65),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;orphans:1;widows:1;width:20px'>Z</p><p style='margin:0;orphans:1;widows:1;width:20px'>W</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y</p></div>",
            &[
                (1, 'W', 55),
                (1, 'X', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;orphans:1;widows:1;width:20px'>Z</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y Y</p></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap-reverse;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y</p></div>",
            &[
                (1, 'W', 55),
                (1, 'X', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column-reverse;flex-wrap:wrap;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y</p></div>",
            &[
                (1, 'Y', 55),
                (1, 'Z', 55),
                (2, 'W', 65),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'X', 65),
                (2, 'Y', 5),
                (2, 'Y', 35),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px'><p style='margin:0;width:20px;order:1'>Z</p><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px;order:2'>W</p><p style='margin:0;orphans:1;widows:1;width:20px;order:2'>Y Y Y</p></div>",
            &[
                (1, 'W', 55),
                (1, 'X', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px;align-items:center'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y</p></div>",
            &[
                (1, 'W', 55),
                (1, 'X', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:200px'><p style='margin:0;width:20px'>Q</p><p style='margin:0;width:20px'>R</p><p style='margin:0;orphans:1;widows:1;width:20px'>S S S S</p><p style='margin:0;width:20px'>W</p></div>",
            &[
                (1, 'Q', 55),
                (1, 'W', 55),
                (2, 'R', 5),
                (2, 'S', 35),
                (2, 'S', 65),
                (3, 'S', 5),
                (3, 'S', 35),
            ],
        ),
    ]);
}
