//! A grid whose items stack one per row, in rows that size to their content,
//! breaks between the lines of an item as a block breaks between the lines
//! of its children: the line that crosses the page end starts the next page,
//! and the items after it move down with it. Where each line falls is what
//! Chromium prints for the same documents on pages 100px tall, with Ahem
//! lines of 20px and a line height of 30px.

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
fn stacked_items_of_an_in_flow_grid_break_between_lines() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><div style='height:30px'>Y</div><p style='margin:0;orphans:1;widows:1'>X X X</p></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid;row-gap:10px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 45),
                (1, 'Z', 75),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0;orphans:1;widows:1'>X X X</p></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'X', 5),
                (2, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><div><p style='margin:0;orphans:1;widows:1'>X X X</p></div><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid;padding-bottom:10px'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Z', 75),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><p style='margin:0;orphans:2;widows:2'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid;grid-template-rows:auto auto'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Z', 65),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><div style='height:20px'>W</div><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'W', 35),
                (0, 'X', 55),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'Y', 65),
                (2, 'Z', 5),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><p style='margin:0;orphans:1;widows:1'>X X X X X X</p><div style='height:30px'>Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'X', 5),
                (2, 'Y', 35),
                (2, 'Z', 65),
            ],
        ),
    ]);
}

#[test]
fn stacked_items_of_an_absolutely_positioned_grid_break_between_lines() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px;display:grid;'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div>",
            &[(1, 'X', 45), (1, 'X', 75), (2, 'X', 5), (2, 'Y', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px;display:grid;'><div style='height:20px'>W</div><p style='margin:0;orphans:1;widows:1'>X X X</p></div>",
            &[(1, 'W', 45), (1, 'X', 65), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px;display:grid;'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0;orphans:1;widows:1'>X X X</p></div>",
            &[
                (1, 'X', 45),
                (1, 'X', 75),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'X', 65),
                (3, 'X', 5),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:140px;width:20px;display:grid;'><p style='margin:0;orphans:3;widows:1'>X X X</p><div style='height:30px'>Y</div></div>",
            &[(1, 'X', 45), (1, 'X', 75), (2, 'X', 5), (2, 'Y', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:20px;display:grid;'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div></div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Y', 65)],
        ),
    ]);
}

#[test]
fn a_padded_first_item_at_the_page_start_keeps_its_first_lines() {
    // The grid starts the first page, so moving its first item gains no room.
    assert_rows(&[(
        "<div style='width:20px;display:grid'><div style='padding-top:15px'><p style='margin:0;orphans:3;widows:1'>X X X</p></div><div style='height:10px'></div></div><div style='height:30px'>Z</div>",
        &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 45)],
    )]);
}

#[test]
fn text_items_follow_and_absolute_children_stay_at_the_grid_start() {
    assert_rows(&[
        // An item of bare text moves down with the rows above it.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:20px;display:grid'><p style='margin:0;orphans:1;widows:1'>X X X</p>Y</div>",
            &[(1, 'X', 55), (2, 'X', 5), (2, 'X', 35), (2, 'Y', 65)],
        ),
        // A child positioned at its static place stays at the top of the grid.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:20px;display:grid'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div><div style='position:absolute;height:30px'>A</div></div>",
            &[
                (1, 'A', 55),
                (1, 'X', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 65),
            ],
        ),
        // A child aligned to the end of the grid follows the grid's end.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:20px;display:grid'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='height:30px'>Y</div><div style='position:absolute;align-self:end;height:30px'>E</div></div>",
            &[
                (1, 'X', 55),
                (2, 'E', 65),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 65),
            ],
        ),
        // So does one at the end of the cross axis of a row flex container.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:20px;display:flex;align-items:end'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='position:absolute;height:30px'>E</div></div>",
            &[(1, 'X', 55), (2, 'E', 35), (2, 'X', 5), (2, 'X', 35)],
        ),
        // In a column flex container the start of the main axis places it.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:20px;display:flex;flex-direction:column'><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='position:absolute;align-self:end;height:30px'>E</div></div>",
            &[(1, 'E', 55), (1, 'X', 55), (2, 'X', 5), (2, 'X', 35)],
        ),
        // A child at the bottom of the grid moves with the grid's end.
        (
            "<div style='height:30px'></div><div style='position:absolute;top:150px;width:20px;display:grid'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0;orphans:1;widows:1'>X X X</p><div style='position:absolute;bottom:0;height:30px'>B</div></div>",
            &[
                (1, 'X', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'X', 65),
                (3, 'B', 35),
                (3, 'X', 5),
                (3, 'X', 35),
            ],
        ),
    ]);
}
