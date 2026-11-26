//! Grids whose items have break properties, in columns.
//!
//! A grid row that holds an item with `break-inside: avoid` moves to the next
//! column whole, with the other items of the row, when it does not fit the
//! rest of the column; a row of items that split breaks where the column
//! ends. The expected rectangles are what Chromium reports through
//! `getClientRects()` for each fragment, on a page with an 8px body margin.
//!
//! https://drafts.csswg.org/css-grid-1/#pagination

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of each fragment of the elements with an
/// `id` in `body`, on the first page of 800px by 600px.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            dom.attr(fragment.node(), "id").map(|id| {
                let rect = fragment.rect();
                (id.to_string(), [rect.x, rect.y, rect.width, rect.height])
            })
        })
        .collect()
}

type Expected = &'static [(&'static str, &'static [[f32; 4]])];

const CASES: &[(&str, Expected)] = &[
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0;column-fill:auto'><div id=g style='display:grid;grid-template:50px 100px / 50px'><div id=a style='height:100px'></div><div id=b style='break-inside:avoid'></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 100.0]]),
            ("a", &[[8.0, 8.0, 50.0, 100.0]]),
            ("b", &[[58.0, 8.0, 50.0, 100.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0'><div id=g style='display:grid;grid-template:50px 100px / 25px 25px;grid-template-areas:\"a a\" \"b c\"'><div id=a style='grid-area:a;height:100px'></div><div id=b style='grid-area:b'></div><div id=c style='grid-area:c;break-inside:avoid'></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 100.0]]),
            ("a", &[[8.0, 8.0, 50.0, 100.0]]),
            ("b", &[[58.0, 8.0, 25.0, 100.0]]),
            ("c", &[[83.0, 8.0, 25.0, 100.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0'><div id=g style='display:grid;grid-template:50px 100px / 50px'><div id=a></div><div id=b style='break-inside:avoid'></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 100.0]]),
            ("a", &[[8.0, 8.0, 50.0, 50.0]]),
            ("b", &[[58.0, 8.0, 50.0, 100.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0'><div id=g style='display:grid;grid-template:50px 100px / 50px'><div id=a></div><div id=b style='height:80px;break-inside:avoid'></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 100.0]]),
            ("a", &[[8.0, 8.0, 50.0, 50.0]]),
            ("b", &[[58.0, 8.0, 50.0, 80.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0'><div id=g style='display:grid;grid-template:50px 100px / 50px'><div id=a style='height:100px'></div><div id=b style='break-inside:avoid'></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 100.0]]),
            ("a", &[[8.0, 8.0, 50.0, 100.0]]),
            ("b", &[[58.0, 8.0, 50.0, 100.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0;column-fill:auto'><div id=g style='display:grid;row-gap:10px'><div id=a style='height:40px'></div><div id=b style='height:40px;break-inside:avoid'></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 90.0]]),
            ("a", &[[8.0, 8.0, 50.0, 40.0]]),
            ("b", &[[8.0, 58.0, 50.0, 40.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0;column-fill:auto'><div id=g style='display:grid'><div id=a style='height:30px'></div><div id=b style='height:30px'></div><div id=c style='height:60px;break-inside:avoid'></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 60.0]]),
            ("a", &[[8.0, 8.0, 50.0, 30.0]]),
            ("b", &[[8.0, 38.0, 50.0, 30.0]]),
            ("c", &[[58.0, 8.0, 50.0, 60.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0;column-fill:auto'><div id=g style='display:grid;grid-template-columns:25px 25px'><div id=a style='height:60px'></div><div id=b style='height:30px'></div><div id=c style='height:50px;break-inside:avoid'></div><div id=d style='height:10px'></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 50.0]]),
            ("a", &[[8.0, 8.0, 25.0, 60.0]]),
            ("b", &[[33.0, 8.0, 25.0, 30.0]]),
            ("c", &[[58.0, 8.0, 25.0, 50.0]]),
            ("d", &[[83.0, 8.0, 25.0, 10.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0;column-fill:auto'><div id=g style='display:grid'><div id=a style='height:60px'></div><div id=b style='height:30px;break-before:avoid'></div><div id=c style='height:30px'></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 20.0]]),
            ("a", &[[8.0, 8.0, 50.0, 60.0]]),
            ("b", &[[8.0, 68.0, 50.0, 30.0]]),
            ("c", &[[8.0, 98.0, 50.0, 10.0], [58.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0;column-fill:auto'><div id=p style='height:30px'></div><div id=g style='display:grid'><div id=a style='height:30px'></div><div id=b style='height:60px;break-inside:avoid'></div></div><div id=n style='height:20px'></div></div>",
        &[
            ("p", &[[8.0, 8.0, 50.0, 30.0]]),
            ("g", &[[8.0, 38.0, 50.0, 70.0], [58.0, 8.0, 50.0, 60.0]]),
            ("a", &[[8.0, 38.0, 50.0, 30.0]]),
            ("b", &[[58.0, 8.0, 50.0, 60.0]]),
            ("n", &[[58.0, 68.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div style='width:100px;height:100px;columns:2;column-gap:0;column-fill:auto'><div id=g style='display:grid'><div id=a style='height:60px'></div><div id=b style='break-inside:avoid'><div style='height:50px'></div></div></div></div>",
        &[
            ("g", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 50.0]]),
            ("a", &[[8.0, 8.0, 50.0, 60.0]]),
            ("b", &[[58.0, 8.0, 50.0, 50.0]]),
        ],
    ),
];

#[test]
fn grid_rows_break_between_columns_like_chromium() {
    let mut failures = Vec::new();
    for &(markup, expected) in CASES {
        let got = boxes(markup);
        for &(id, rects) in expected {
            let fragments: Vec<[f32; 4]> = got
                .iter()
                .filter(|(name, _)| name == id)
                .map(|(_, rect)| *rect)
                .collect();
            let matches = fragments.len() == rects.len()
                && fragments
                    .iter()
                    .zip(rects)
                    .all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 0.05));
            if !matches {
                failures.push(format!("{markup} #{id}: got {fragments:?}, want {rects:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
