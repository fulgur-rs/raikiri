//! Items that a flex or grid container inside an absolutely positioned box
//! sets side by side break between their lines at a page end, as they do in
//! the normal flow: the line that crosses the page end starts the next page,
//! the items beside it keep their places, and the rows or flex lines after it
//! move down with the end of its row. A row whose size does not follow its
//! content holds the moved lines while they fit, and a row stretched past its
//! content gives that room first. Where each line falls is what Chromium
//! prints for the same documents on pages 100px tall, with Ahem lines of 20px
//! and a line height of 30px.

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
fn positioned_items_side_by_side_break_between_their_lines() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div>",
            &[(1, 'X', 55), (1, 'Y', 55), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:flex'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y</p></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px'><div style='display:flex'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Y</p></div><div>Z</div></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
            ],
        ),
    ]);
}

#[test]
fn positioned_later_rows_and_lines_follow_the_row_end() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div><div>Z</div></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:flex;flex-wrap:wrap'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Y</p><p style='margin:0;width:40px'>Z</p></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:120px;width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:2'>X X X X</p><div style='height:30px'>Y</div><div>Z</div></div>",
            &[
                (1, 'X', 25),
                (1, 'X', 55),
                (1, 'Y', 25),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:grid;grid-template-columns:20px 20px;row-gap:10px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div>Y</div><div>Z</div><div>Z</div></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 75),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:60px;display:grid;grid-template-columns:20px 20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0;orphans:1;widows:1'>Y Y Y Y</p><div>Z</div><div>Z</div></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (1, 'Z', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
                (3, 'Z', 5),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:flex;flex-wrap:wrap;padding-bottom:10px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Y</p><p style='margin:0;width:40px'>Z</p><p style='margin:0;width:40px'>Z</p></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
                (3, 'Z', 5),
            ],
        ),
        // Rows and lines follow one another in the order of their items.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:flex;flex-wrap:wrap'><p style='margin:0;width:40px;order:1'>Z</p><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Y</p></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:grid;grid-template-columns:20px 20px'><div style='grid-row:2;grid-column:1'>Z</div><p style='margin:0;orphans:1;widows:1;grid-row:1'>X X X</p><div>Y</div></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
            ],
        ),
    ]);
}

#[test]
fn positioned_rows_not_sized_by_their_content_hold_the_moved_lines() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:grid;grid-template-columns:20px 20px;min-height:150px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div>",
            &[(1, 'X', 55), (1, 'Y', 55), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:grid;grid-template-columns:20px 20px;grid-auto-rows:100px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div><div>Z</div></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 55),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:grid;grid-template-columns:20px 20px;align-items:start'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div><div>Z</div></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
            ],
        ),
        // An item stretched to the row takes its growth in the room first.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div><p style='margin:0;orphans:1;widows:1'>Y Y Y</p></div><div>Z</div></div>",
            &[
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Z', 65),
            ],
        ),
    ]);
}

#[test]
fn positioned_column_flex_items_in_one_column_stack() {
    // The items after the grown one in its column follow it; the items of
    // another column stay.
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:flex;flex-direction:column'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0'>Z</p></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 65)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Z', 65)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:flex;flex-direction:column-reverse'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0'>Z</p></div>",
            &[(1, 'Z', 55), (2, 'X', 5), (2, 'X', 35), (2, 'X', 65)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:90px'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(1, 'X', 55), (1, 'Z', 55), (2, 'X', 5), (2, 'X', 35)],
        ),
    ]);
}
