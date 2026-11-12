//! A flex container whose items run down the page in columns that wrap
//! breaks its columns against its height. Where that height is `auto` and
//! only `max-height` limits it, the columns break against the `max-height`,
//! in an absolutely positioned container as in one in the flow. Where each
//! line falls is what Chromium prints for the same documents on pages 100px
//! tall, with Ahem lines of 20px and a line height of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The laid-out pages of `body` on pages `page` tall.
fn pages(body: &str, page: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:{page};margin:0}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
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

/// The page, glyph and glyph top of every line of text in `body`.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let result = pages(body, "100px 100px");
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

/// The id and border box of every box with an id in `body`, laid out on one
/// page.
fn boxes(body: &str) -> Vec<(String, [i32; 4])> {
    let result = pages(body, "1000px 1000px");
    let page = result.page(0).unwrap();
    page.fragments()
        .filter_map(|fragment| {
            let id = page.dom().attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((
                id.to_owned(),
                [rect.x, rect.y, rect.width, rect.height].map(|value| value.round() as i32),
            ))
        })
        .collect()
}

/// A document body and the page, glyph and top of each line.
type Row = (&'static str, &'static [(u32, char, i32)]);

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}

#[test]
fn a_positioned_column_container_wraps_its_items_against_its_max_height() {
    // Items of 90px, 30px, 30px and 90px make two columns of 120px. The
    // container is absolutely positioned at its static position or at
    // `top: 0`, with a larger `max-height`, a `min-height` below it or a
    // percentage one, with padding, gaps, a distribution of its lines or of
    // its items, or with its lines in reverse; one with only a `min-height`
    // keeps a single column.
    assert_rows(&[
        (
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
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
            "<div style='height:0px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (0, 'E', 5),
                (0, 'W', 5),
                (0, 'X', 5),
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'Y', 5),
                (1, 'Z', 5),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='position:absolute;top:0;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (0, 'W', 5),
                (0, 'X', 5),
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'E', 55),
                (1, 'Y', 5),
                (1, 'Z', 5),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='position:absolute;top:150px;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
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
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;height:120px'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
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
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px;row-gap:10px;align-content:space-between'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
                (1, 'X', 55),
                (1, 'Y', 55),
                (1, 'Z', 55),
                (2, 'W', 5),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px;justify-content:flex-end'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
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
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px;flex-wrap:wrap-reverse'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
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
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px;padding:10px;box-sizing:border-box'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
                (1, 'X', 65),
                (1, 'Y', 65),
                (1, 'Z', 65),
                (2, 'W', 5),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px;padding:10px'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
                (1, 'W', 65),
                (1, 'X', 65),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Y', 65),
                (2, 'Z', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:150px'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
                (1, 'X', 55),
                (1, 'Y', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
                (2, 'Z', 65),
                (3, 'W', 5),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:120px;min-height:50px'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
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
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;min-height:120px'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
                (1, 'X', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
                (3, 'W', 5),
                (3, 'Y', 35),
                (3, 'Y', 65),
                (4, 'Y', 5),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='position:absolute;width:40px;display:flex;flex-direction:column;flex-wrap:wrap;max-height:50%'><p style='margin:0;width:20px;orphans:1;widows:1'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px;orphans:1;widows:1'>Y Y Y</p></div><p style='margin:0'>E</p>",
            &[
                (1, 'E', 55),
                (1, 'W', 55),
                (1, 'X', 55),
                (1, 'Y', 55),
                (1, 'Z', 55),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Y', 5),
                (2, 'Y', 35),
            ],
        ),
    ]);
}

/// A document body and the id and border box of each box in it.
type BoxRow = (&'static str, &'static [(&'static str, [i32; 4])]);

fn assert_boxes(rows: &[BoxRow]) {
    for &(body, expected) in rows {
        let expected = expected
            .iter()
            .map(|&(id, rect)| (id.to_owned(), rect))
            .collect::<Vec<_>>();
        assert_eq!(boxes(body), expected, "{body}");
    }
}

/// Items of 90px, 30px, 30px and 90px down the page.
const COLUMN_ITEMS: &str = "<p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p>";

/// Items of 60px, 20px, 20px and 60px down the page.
const ROW_ITEMS: &str = "<p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:20px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p>";

#[test]
fn the_wrapped_lines_of_a_positioned_container_sit_side_by_side() {
    // A column container puts its first line on the left, or on the right
    // when its lines run in reverse. In a vertical writing mode the items of
    // a row container run down the page and its first line is on the right,
    // positioned or floated, and with a stated width the lines share it.
    let column = |style: &str| {
        format!(
            "<div id=c style='position:absolute;width:40px;display:flex;flex-direction:column;{style}'>{COLUMN_ITEMS}</div>"
        )
    };
    let row = |style: &str| {
        format!(
            "<div id=c style='{style}writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px'>{ROW_ITEMS}</div>"
        )
    };
    let bodies = [
        column("flex-wrap:wrap;max-height:120px"),
        column("flex-wrap:wrap-reverse;max-height:120px"),
        row("position:absolute;"),
        row("float:left;"),
        row("position:absolute;width:100px;"),
    ];
    let rows: [&[(&str, [i32; 4])]; 5] = [
        &[
            ("c", [0, 0, 40, 120]),
            ("a", [0, 0, 20, 90]),
            ("b", [0, 90, 20, 30]),
            ("w", [20, 0, 20, 30]),
            ("d", [20, 30, 20, 90]),
        ],
        &[
            ("c", [0, 0, 40, 120]),
            ("a", [20, 0, 20, 90]),
            ("b", [20, 90, 20, 30]),
            ("w", [0, 0, 20, 30]),
            ("d", [0, 30, 20, 90]),
        ],
        &[
            ("c", [0, 0, 40, 80]),
            ("a", [20, 0, 20, 60]),
            ("b", [20, 60, 20, 20]),
            ("w", [0, 0, 20, 20]),
            ("d", [0, 20, 20, 60]),
        ],
        &[
            ("c", [0, 0, 40, 80]),
            ("a", [20, 0, 20, 60]),
            ("b", [20, 60, 20, 20]),
            ("w", [0, 0, 20, 20]),
            ("d", [0, 20, 20, 60]),
        ],
        &[
            ("c", [0, 0, 100, 80]),
            ("a", [80, 0, 20, 60]),
            ("b", [80, 60, 20, 20]),
            ("w", [30, 0, 20, 20]),
            ("d", [30, 20, 20, 60]),
        ],
    ];
    for (body, expected) in bodies.iter().zip(rows) {
        let expected = expected
            .iter()
            .map(|&(id, rect)| (id.to_owned(), rect))
            .collect::<Vec<_>>();
        assert_eq!(boxes(body), expected, "{body}");
    }
}

#[test]
fn a_wrapping_column_item_keeps_the_height_its_container_grows_it_to() {
    // The container's height is not definite, and the item it grows fills
    // it with one line of items.
    assert_boxes(&[(
        "<div style='display:flex;flex-direction:column;min-height:400px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;flex-grow:1;width:40px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
        &[
            ("c", [0, 0, 40, 400]),
            ("a", [0, 0, 20, 90]),
            ("b", [0, 90, 20, 30]),
            ("w", [0, 120, 20, 30]),
            ("d", [0, 150, 20, 90]),
        ],
    )]);
}
