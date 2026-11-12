//! A flex container that wraps its items onto lines along its block axis is,
//! at its min-content block size, as long as its items on a single line: the
//! min-content block size of a box is the block size it lays out to. A grid
//! row or other track sized by that contribution keeps the container on one
//! line, where only a `min-height`, a `max-height` or a zero minimum lets the
//! container shrink and wrap. Where each box falls is what Chromium lays out
//! for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A document body and the id and border box of each box with an id in it.
type Row = (&'static str, &'static [(&'static str, [i32; 4])]);

/// The id and border box of every box with an id in `body`, laid out on one
/// page 1280px wide.
fn boxes(body: &str) -> Vec<(String, [i32; 4])> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:1280px 1000px;margin:0}} body{{margin:0}}</style><body>{body}</body>"
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

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        let expected = expected
            .iter()
            .map(|&(id, rect)| (id.to_owned(), rect))
            .collect::<Vec<_>>();
        assert_eq!(boxes(body), expected, "{body}");
    }
}

#[test]
fn a_wrapping_column_contributes_its_items_on_one_line_to_the_height_of_its_track() {
    // Items of 90px, 30px, 30px and 90px in a column 40px wide make a grid
    // row 240px tall in a grid only 100px tall, whether the row is `auto`,
    // `min-content`, `max-content` or the grid has no rows of its own, the
    // item starts its row or stretches to it, and with gaps, padding, a
    // border or items in reverse. A zero minimum (`min-height: 0` or a
    // `minmax(0, auto)` row) lets the row shrink and the column wrap, as do
    // the column's own `min-height` and `max-height`. The same holds for an
    // inline column that fills its grid area, a column whose height is
    // `min-content`, a grid stretched in a row flex container, and a column
    // flex container 100px tall, which leaves the column's automatic minimum
    // height at its items on one line unless `min-height` is 0.
    assert_rows(&[
        (
            "<div id=g style='display:grid;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 240]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [0, 150, 20, 90]),
                ("n", [0, 240, 1280, 5]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px;align-items:start'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 240]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [0, 150, 20, 90]),
                ("n", [0, 240, 1280, 5]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;height:auto;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 95]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
                ("n", [0, 95, 1280, 5]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px;grid-template-rows:minmax(0,auto)'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px;grid-template-rows:min-content 1fr'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 240]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [0, 150, 20, 90]),
                ("n", [0, 240, 1280, 5]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px;grid-template-rows:max-content'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;row-gap:10px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 270]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 100, 20, 30]),
                ("w", [0, 140, 20, 30]),
                ("d", [0, 180, 20, 90]),
                ("n", [0, 270, 1280, 5]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;padding-top:10px;border-bottom:3px solid'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 253]),
                ("a", [0, 10, 20, 90]),
                ("b", [0, 100, 20, 30]),
                ("w", [0, 130, 20, 30]),
                ("d", [0, 160, 20, 90]),
                ("n", [0, 253, 1280, 5]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:120px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 120]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [20, 0, 20, 30]),
                ("d", [20, 30, 20, 90]),
                ("n", [0, 120, 1280, 5]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px;grid-template-columns:auto auto'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;flex-direction:column-reverse'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 240]),
                ("a", [0, 150, 20, 90]),
                ("b", [0, 120, 20, 30]),
                ("w", [0, 90, 20, 30]),
                ("d", [0, 0, 20, 90]),
                ("n", [660, 0, 620, 5]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px'><div id=c style='display:inline-flex;flex-direction:column;flex-wrap:wrap'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 1280, 240]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [0, 150, 20, 90]),
                ("n", [0, 240, 1280, 5]),
            ],
        ),
        (
            "<div id=g style='display:flex;height:100px;align-items:flex-start'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;height:min-content'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 240]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [0, 150, 20, 90]),
            ],
        ),
        (
            "<div style='display:flex;height:100px'><div id=g style='display:grid'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div></div>",
            &[
                ("g", [0, 0, 40, 100]),
                ("c", [0, 0, 40, 240]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [0, 150, 20, 90]),
            ],
        ),
        (
            "<div id=g style='display:grid;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;max-height:150px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div><div id=n style='height:5px'></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 150]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [20, 0, 20, 90]),
                ("n", [0, 150, 1280, 5]),
            ],
        ),
        (
            "<div id=g style='display:flex;flex-direction:column;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 240]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [0, 150, 20, 90]),
            ],
        ),
        (
            "<div id=g style='display:flex;flex-direction:column;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("g", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_wrapping_column_contributes_its_items_on_one_line_to_the_width_of_its_track() {
    // In vertical-rl and vertical-lr, the column of items 90px, 30px, 30px
    // and 90px wide is 240px wide in a grid only 100px wide, and wraps
    // against its own `max-width` of 150px.
    assert_rows(&[
        (
            "<div id=g style='writing-mode:vertical-rl;display:grid;width:100px;height:300px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;height:40px'><p id=a style='margin:0;width:90px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:30px;height:20px'></p><p id=d style='margin:0;width:90px;height:20px'></p></div><div id=n style='width:5px'></div></div>",
            &[
                ("g", [0, 0, 100, 300]),
                ("c", [-140, 0, 240, 40]),
                ("a", [10, 0, 90, 20]),
                ("b", [-20, 0, 30, 20]),
                ("w", [-50, 0, 30, 20]),
                ("d", [-140, 0, 90, 20]),
                ("n", [-145, 0, 5, 300]),
            ],
        ),
        (
            "<div id=g style='writing-mode:vertical-rl;display:grid;width:100px;height:300px;align-items:start'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;height:40px;max-width:150px'><p id=a style='margin:0;width:90px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:30px;height:20px'></p><p id=d style='margin:0;width:90px;height:20px'></p></div><div id=n style='width:5px'></div></div>",
            &[
                ("g", [0, 0, 100, 300]),
                ("c", [-50, 0, 150, 40]),
                ("a", [10, 0, 90, 20]),
                ("b", [-20, 0, 30, 20]),
                ("w", [-50, 0, 30, 20]),
                ("d", [10, 20, 90, 20]),
                ("n", [-55, 0, 5, 300]),
            ],
        ),
        (
            "<div id=g style='writing-mode:vertical-lr;display:grid;width:100px;height:300px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;height:40px'><p id=a style='margin:0;width:90px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:30px;height:20px'></p><p id=d style='margin:0;width:90px;height:20px'></p></div><div id=n style='width:5px'></div></div>",
            &[
                ("g", [0, 0, 100, 300]),
                ("c", [0, 0, 240, 40]),
                ("a", [0, 0, 90, 20]),
                ("b", [90, 0, 30, 20]),
                ("w", [120, 0, 30, 20]),
                ("d", [150, 0, 90, 20]),
                ("n", [240, 0, 5, 300]),
            ],
        ),
    ]);
}
