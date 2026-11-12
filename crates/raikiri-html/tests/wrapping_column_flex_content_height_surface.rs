//! A flex container that wraps its items onto lines along its block axis, a
//! column one in a horizontal writing mode, and whose height its content
//! decides breaks its lines against its `max-height` only, not against the
//! room around it, and is as tall as its longest line within its limits. That
//! holds where it is absolutely positioned, floats, is inline, or is a grid
//! or flex item that its parent does not stretch. Where each box falls is what
//! Chromium lays out for the same documents.

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
fn a_wrapping_column_is_as_tall_as_its_longest_line() {
    // Items 60px, 30px, 20px and 60px tall, or 40px, 40px and 10px, broken
    // into columns by a `max-height` of 80px with or without padding, as a
    // content box or a border box. The room around the container, the 400px of
    // a containing block or the 300px of a grid area or a flex line, neither
    // breaks the lines nor stretches them; a `min-height`, a percentage item
    // height, flexible items and `wrap-reverse` keep their meaning.
    assert_rows(&[
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [40, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px;padding:5px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 70, 70]),
                ("a", [5, 5, 20, 60]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 20]),
                ("d", [45, 5, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px;padding:5px;box-sizing:border-box'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 70, 70]),
                ("a", [5, 5, 20, 60]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 20]),
                ("d", [45, 5, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-direction:column;flex-wrap:wrap;max-width:80px'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 60]),
                ("a", [0, 0, 60, 20]),
                ("b", [30, 20, 30, 20]),
                ("w", [10, 20, 20, 20]),
                ("d", [0, 40, 60, 20]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-direction:column;flex-wrap:wrap;max-width:80px;padding:5px'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 70, 70]),
                ("a", [5, 5, 60, 20]),
                ("b", [35, 25, 30, 20]),
                ("w", [15, 25, 20, 20]),
                ("d", [5, 45, 60, 20]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='float:left;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [40, 0, 20, 60]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [200, 0, 20, 30]),
                ("w", [200, 30, 20, 20]),
                ("d", [400, 0, 20, 60]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='display:inline-flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [40, 0, 20, 60]),
            ],
        ),
        (
            "<div style='display:grid;width:600px;height:300px;align-items:start'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [200, 0, 20, 30]),
                ("w", [200, 30, 20, 20]),
                ("d", [400, 0, 20, 60]),
            ],
        ),
        (
            "<div style='display:flex;width:600px;height:300px;align-items:start'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [40, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:200px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 20, 170]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("w", [0, 90, 20, 20]),
                ("d", [0, 110, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;height:auto;max-height:50%'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 20, 170]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("w", [0, 90, 20, 20]),
                ("d", [0, 110, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:100px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;top:0;bottom:0'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("w", [20, 0, 20, 20]),
                ("d", [20, 20, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px;padding:5px'><p id=a style='margin:0;height:40px;width:20px'></p><p id=b style='margin:0;height:40px;width:20px'></p><p id=w style='margin:0;height:10px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 50, 90]),
                ("a", [5, 5, 20, 40]),
                ("b", [5, 45, 20, 40]),
                ("w", [25, 5, 20, 10]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px;padding:5px;box-sizing:border-box'><p id=a style='margin:0;height:40px;width:20px'></p><p id=b style='margin:0;height:40px;width:20px'></p><p id=w style='margin:0;height:10px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 50, 60]),
                ("a", [5, 5, 20, 40]),
                ("b", [25, 5, 20, 40]),
                ("w", [25, 45, 20, 10]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:100px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 20, 170]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("w", [0, 90, 20, 20]),
                ("d", [0, 110, 20, 60]),
            ],
        ),
        (
            "<div style='display:flex;width:600px;height:300px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 80]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [40, 0, 20, 60]),
            ],
        ),
        (
            "<div style='display:flex;width:600px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div><div id=s style='width:20px;height:150px'></div></div>",
            &[
                ("c", [0, 0, 60, 80]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [40, 0, 20, 60]),
                ("s", [60, 0, 20, 150]),
            ],
        ),
        (
            "<div style='display:grid;width:600px;height:300px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 80]),
                ("a", [0, 0, 20, 60]),
                ("b", [200, 0, 20, 30]),
                ("w", [200, 30, 20, 20]),
                ("d", [400, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px;min-height:75px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 75]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [40, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 40, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 0]),
                ("d", [20, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px;align-content:space-between'><p id=a style='margin:0;height:60px;width:20px;flex-grow:1'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:40px;width:20px;flex-grow:1'></p></div></div>",
            &[
                ("c", [0, 0, 40, 70]),
                ("a", [0, 0, 20, 70]),
                ("b", [20, 0, 20, 30]),
                ("d", [20, 30, 20, 40]),
            ],
        ),
        (
            "<div style='width:600px;height:300px;display:flex;flex-direction:column'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [200, 0, 20, 30]),
                ("w", [200, 30, 20, 20]),
                ("d", [400, 0, 20, 60]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='display:inline-flex;flex-direction:column;flex-wrap:wrap;max-height:80px;padding:5px'><p id=a style='margin:0;height:40px;width:20px'></p><p id=b style='margin:0;height:40px;width:20px'></p><p id=w style='margin:0;height:10px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 50, 90]),
                ("a", [5, 5, 20, 40]),
                ("b", [5, 45, 20, 40]),
                ("w", [25, 5, 20, 10]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap-reverse;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 60]),
                ("a", [40, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [0, 0, 20, 60]),
            ],
        ),
    ]);
}

#[test]
fn a_wrapping_column_keeps_the_height_it_is_given() {
    // The same items. A column its own column flex container flexes, a
    // `fit-content` or `max-content` height, a vertical column in a table
    // cell and an in-flow column in a block with a definite height break their
    // lines as Chromium does; a column measured inside a float stays on one
    // line where its container gives it no height to break against.
    assert_rows(&[
        (
            "<div style='width:600px'><div id=f style='float:left;display:flex;flex-direction:column'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;flex-basis:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div></div>",
            &[
                ("f", [0, 0, 20, 170]),
                ("c", [0, 0, 20, 170]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("w", [0, 90, 20, 20]),
                ("d", [0, 110, 20, 60]),
            ],
        ),
        (
            "<div style='width:600px'><div id=f style='float:left;display:flex;flex-direction:column;height:80px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div></div>",
            &[
                ("f", [0, 0, 20, 80]),
                ("c", [0, 0, 20, 170]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("w", [0, 90, 20, 20]),
                ("d", [0, 110, 20, 60]),
            ],
        ),
        (
            "<div style='width:600px'><div id=f style='float:left;display:flex;flex-direction:column;max-height:80px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;min-height:0'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div></div>",
            &[
                ("f", [0, 0, 20, 80]),
                ("c", [0, 0, 20, 80]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [40, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;height:fit-content;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 20]),
                ("d", [40, 0, 20, 60]),
            ],
        ),
        (
            "<div style='width:600px;height:300px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;height:fit-content;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [200, 0, 20, 30]),
                ("w", [200, 30, 20, 20]),
                ("d", [400, 0, 20, 60]),
            ],
        ),
        (
            "<div style='width:600px;height:300px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;height:max-content;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [200, 0, 20, 30]),
                ("w", [200, 30, 20, 20]),
                ("d", [400, 0, 20, 60]),
            ],
        ),
        (
            "<table><tr><td id=t><div id=c style='writing-mode:vertical-rl;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></td></tr></table>",
            &[
                ("t", [2, 2, 82, 62]),
                ("c", [3, 3, 80, 60]),
                ("a", [63, 3, 20, 60]),
                ("b", [43, 3, 20, 30]),
                ("w", [23, 3, 20, 20]),
                ("d", [3, 3, 20, 60]),
            ],
        ),
        (
            "<div style='width:600px;height:300px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 60]),
                ("a", [0, 0, 20, 60]),
                ("b", [200, 0, 20, 30]),
                ("w", [200, 30, 20, 20]),
                ("d", [400, 0, 20, 60]),
            ],
        ),
        (
            "<div style='width:600px;'><div id=f style='display:flex;flex-direction:column;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;flex:1'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div></div>",
            &[
                ("f", [0, 0, 600, 100]),
                ("c", [0, 0, 600, 170]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("w", [0, 90, 20, 20]),
                ("d", [0, 110, 20, 60]),
            ],
        ),
    ]);
}
