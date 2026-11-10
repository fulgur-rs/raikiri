//! Items that a flex or grid container sets side by side break between their
//! lines at a page end, as a block breaks between the lines of its children:
//! the line that crosses the page end starts the next page, the items beside
//! it keep their places, and the rows or flex lines after it move down with
//! the end of its row. A row whose size does not follow its content holds the
//! moved lines while they fit, and a row stretched past its content gives
//! that room first. Where each line falls is what Chromium prints for the
//! same documents on pages 100px tall, with Ahem lines of 20px and a line
//! height of 30px.

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
fn items_side_by_side_break_between_their_lines() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0'>Y Y Y</p></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'X', 5),
                (1, 'Y', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px;display:grid'><div style='display:flex'><p style='margin:0;width:20px'>X X X</p><p style='margin:0;width:20px'>Y Y Y</p></div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'X', 5),
                (1, 'Y', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px;display:flex'><p style='margin:0;width:20px'>X X X</p><p style='margin:0;width:20px'>Y Y Y</p></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'X', 5),
                (1, 'Y', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px'><div style='display:flex'><p style='margin:0;width:20px'>X X X</p><p style='margin:0;width:20px'>Y Y Y</p></div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'X', 5),
                (1, 'Y', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0;orphans:1;widows:1'>Y Y Y Y</p></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'X', 5),
                (1, 'Y', 5),
                (1, 'Y', 35),
                (1, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:flex'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><div style='width:20px;height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div><div style='grid-column:1/3;height:30px'>W</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'W', 35),
                (1, 'X', 5),
                (1, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:flex'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><div style='width:20px;background:red'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:flex;flex-wrap:wrap'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y</p><p style='margin:0;orphans:1;widows:1;width:40px'>W</p></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'W', 35),
                (1, 'X', 5),
                (1, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0;orphans:1;widows:1'>Y Y Y Y Y</p><div style='height:30px'>W</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'X', 5),
                (1, 'Y', 5),
                (1, 'Y', 35),
                (1, 'Y', 65),
                (2, 'W', 5),
                (2, 'Z', 35),
            ],
        ),
        (
            "<div style='height:10px'></div><div style='width:40px;display:flex'><div><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p></div><div style='width:20px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 15),
                (0, 'X', 45),
                (0, 'X', 75),
                (0, 'Y', 15),
                (1, 'Z', 5),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px;display:grid;grid-template-columns:20px 20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0;orphans:1;widows:1'>Y</p><p style='margin:0;orphans:1;widows:1'>W W W W</p></div><div style='height:30px'>Z</div>",
            &[
                (0, 'W', 35),
                (0, 'W', 65),
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'W', 5),
                (1, 'W', 35),
                (1, 'X', 5),
                (1, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:2;widows:2'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px'><div style='display:flex'><p style='margin:0;orphans:1;widows:1;width:20px'>X X X</p><p style='margin:0;orphans:1;widows:1;width:20px'>Y Y Y</p></div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'X', 5),
                (1, 'Y', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1'>X X X X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'X', 5),
                (2, 'Z', 35),
            ],
        ),
    ]);
}

#[test]
fn closing_edges_and_gaps_follow_the_row_end() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px'><p style='margin:0;orphans:1;widows:1;padding-bottom:20px'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'Z', 55),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px;padding-bottom:10px;border-bottom:5px solid'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:40px;display:grid;grid-template-columns:20px 20px;row-gap:10px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div><div>W</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'W', 45),
                (1, 'X', 5),
                (1, 'Z', 75),
            ],
        ),
    ]);
}

#[test]
fn rows_not_sized_by_their_content_hold_the_moved_lines() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid;grid-auto-rows:100px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (2, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid;min-height:130px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid;height:130px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Z', 65),
            ],
        ),
    ]);
}
