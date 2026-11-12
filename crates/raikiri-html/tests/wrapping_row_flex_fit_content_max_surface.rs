//! A flex container that wraps its items onto lines along its inline axis and
//! sizes that axis to fit its content (it floats, is absolutely positioned or
//! inline, or is a grid item that does not stretch) is as long as its items on
//! one line, within the room it has and its own limits. A maximum given as a
//! border box, or one the padding adds to, does not shorten that length, and
//! the lines then break against the content box. Where each box falls is what
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
fn a_wrapping_row_fits_its_width_to_its_items_within_its_limits() {
    // Items 60px, 30px, 20px and 60px wide in a row with 5px of padding: 100px
    // of room makes two lines. A `max-width` of 80px makes three lines in a
    // box 90px wide, or 80px wide as a border box, whether it is positioned,
    // floats, is inline or a grid item; a `min-width` raises it, a percentage
    // maximum resolves against the containing block, and a border, a gap or a
    // generous maximum change no more than the room the lines have.
    assert_rows(&[
        (
            "<div style='position:relative;width:100px;height:100px'><div id=c style='position:absolute;display:flex;flex-wrap:wrap;padding:5px'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 100, 50]),
                ("a", [5, 5, 60, 20]),
                ("b", [65, 5, 30, 20]),
                ("w", [5, 25, 20, 20]),
                ("d", [25, 25, 60, 20]),
            ],
        ),
        (
            "<div style='position:relative;width:100px;height:100px'><div id=c style='position:absolute;display:flex;flex-wrap:wrap;padding:5px;max-width:80px'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 90, 70]),
                ("a", [5, 5, 60, 20]),
                ("b", [5, 25, 30, 20]),
                ("w", [35, 25, 20, 20]),
                ("d", [5, 45, 60, 20]),
            ],
        ),
        (
            "<div style='position:relative;width:100px;height:100px'><div id=c style='position:absolute;display:flex;flex-wrap:wrap;padding:5px;max-width:80px;box-sizing:border-box'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 80, 70]),
                ("a", [5, 5, 60, 20]),
                ("b", [5, 25, 30, 20]),
                ("w", [35, 25, 20, 20]),
                ("d", [5, 45, 60, 20]),
            ],
        ),
        (
            "<div style='width:100px'><div id=c style='float:left;display:flex;flex-wrap:wrap;padding:5px'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 100, 50]),
                ("a", [5, 5, 60, 20]),
                ("b", [65, 5, 30, 20]),
                ("w", [5, 25, 20, 20]),
                ("d", [25, 25, 60, 20]),
            ],
        ),
        (
            "<div style='width:100px'><div id=c style='display:inline-flex;flex-wrap:wrap;padding:5px'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 100, 50]),
                ("a", [5, 5, 60, 20]),
                ("b", [65, 5, 30, 20]),
                ("w", [5, 25, 20, 20]),
                ("d", [25, 25, 60, 20]),
            ],
        ),
        (
            "<div style='width:300px;height:300px'><div id=c style='float:left;display:flex;flex-wrap:wrap;padding:5px;max-width:80px;box-sizing:border-box'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 80, 70]),
                ("a", [5, 5, 60, 20]),
                ("b", [5, 25, 30, 20]),
                ("w", [35, 25, 20, 20]),
                ("d", [5, 45, 60, 20]),
            ],
        ),
        (
            "<div style='width:300px'><div id=c style='display:inline-flex;flex-wrap:wrap;padding:5px;max-width:80px;box-sizing:border-box'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 80, 70]),
                ("a", [5, 5, 60, 20]),
                ("b", [5, 25, 30, 20]),
                ("w", [35, 25, 20, 20]),
                ("d", [5, 45, 60, 20]),
            ],
        ),
        (
            "<div style='width:300px;height:300px'><div id=c style='float:left;display:flex;flex-wrap:wrap;padding:5px;max-width:80px;min-width:90px;box-sizing:border-box'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 90, 70]),
                ("a", [5, 5, 60, 20]),
                ("b", [5, 25, 30, 20]),
                ("w", [35, 25, 20, 20]),
                ("d", [5, 45, 60, 20]),
            ],
        ),
        (
            "<div style='width:300px;height:300px'><div id=c style='float:left;display:flex;flex-wrap:wrap;padding:5px 7px;border:3px solid;max-width:100px'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 120, 56]),
                ("a", [10, 8, 60, 20]),
                ("b", [70, 8, 30, 20]),
                ("w", [10, 28, 20, 20]),
                ("d", [30, 28, 60, 20]),
            ],
        ),
        (
            "<div style='width:300px;height:300px'><div id=c style='float:left;display:flex;flex-wrap:wrap;padding:5px;max-width:30%;box-sizing:border-box'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 90, 70]),
                ("a", [5, 5, 60, 20]),
                ("b", [5, 25, 30, 20]),
                ("w", [35, 25, 20, 20]),
                ("d", [5, 45, 60, 20]),
            ],
        ),
        (
            "<div style='width:300px;height:300px'><div id=c style='float:left;display:flex;flex-wrap:wrap;padding:5px;max-width:80px;box-sizing:border-box;gap:4px'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 80, 78]),
                ("a", [5, 5, 60, 20]),
                ("b", [5, 29, 30, 20]),
                ("w", [39, 29, 20, 20]),
                ("d", [5, 53, 60, 20]),
            ],
        ),
        (
            "<div style='position:relative;width:300px;height:300px'><div id=c style='position:absolute;display:flex;flex-wrap:wrap;padding:5px;max-width:400px;box-sizing:border-box'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 180, 30]),
                ("a", [5, 5, 60, 20]),
                ("b", [65, 5, 30, 20]),
                ("w", [95, 5, 20, 20]),
                ("d", [115, 5, 60, 20]),
            ],
        ),
        (
            "<div style='display:grid;width:300px;justify-items:start'><div id=c style='display:flex;flex-wrap:wrap;padding:5px;max-width:80px;box-sizing:border-box'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:20px;height:20px'></p><p id=d style='margin:0;width:60px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 80, 70]),
                ("a", [5, 5, 60, 20]),
                ("b", [5, 25, 30, 20]),
                ("w", [35, 25, 20, 20]),
                ("d", [5, 45, 60, 20]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_wrapping_row_fits_its_height_to_its_items_within_its_limits() {
    // The same in a vertical writing mode, where the height is the inline
    // size: a `max-height` of 80px gives a box 90px tall, or 80px as a border
    // box, positioned or floating, in vertical-rl or vertical-lr, with lines in
    // reverse; a `min-height` raises it, and with no limit the box is as tall as
    // its room or its items on one line. A row that does not wrap shrinks its
    // items instead.
    assert_rows(&[
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px;box-sizing:border-box;padding:5px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 70, 80]),
                ("a", [45, 5, 20, 60]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 20]),
                ("d", [5, 5, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px;padding:5px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 70, 90]),
                ("a", [45, 5, 20, 60]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 20]),
                ("d", [5, 5, 20, 60]),
            ],
        ),
        (
            "<div id=c style='float:left;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px;box-sizing:border-box;padding:5px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div>",
            &[
                ("c", [0, 0, 70, 80]),
                ("a", [45, 5, 20, 60]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 20]),
                ("d", [5, 5, 20, 60]),
            ],
        ),
        (
            "<div id=c style='writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px;box-sizing:border-box;padding:5px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div>",
            &[
                ("c", [0, 0, 70, 80]),
                ("a", [45, 5, 20, 60]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 20]),
                ("d", [5, 5, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:100px;height:100px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;padding:5px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 50, 100]),
                ("a", [25, 5, 20, 60]),
                ("b", [25, 65, 20, 30]),
                ("w", [5, 5, 20, 20]),
                ("d", [5, 25, 20, 60]),
            ],
        ),
        (
            "<div style='writing-mode:vertical-rl;height:100px'><div id=c style='float:left;display:flex;flex-wrap:wrap;padding:5px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 50, 100]),
                ("a", [25, 5, 20, 60]),
                ("b", [25, 65, 20, 30]),
                ("w", [5, 5, 20, 20]),
                ("d", [5, 25, 20, 60]),
            ],
        ),
        (
            "<div style='width:300px;height:300px'><div id=c style='float:left;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;padding:5px;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 70, 90]),
                ("a", [45, 5, 20, 60]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 20]),
                ("d", [5, 5, 20, 60]),
            ],
        ),
        (
            "<div style='width:300px;height:300px'><div id=c style='float:left;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;padding:5px;max-height:80px;min-height:100px;box-sizing:border-box'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 50, 100]),
                ("a", [25, 5, 20, 60]),
                ("b", [25, 65, 20, 30]),
                ("w", [5, 5, 20, 20]),
                ("d", [5, 25, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:300px;height:300px'><div id=c style='position:absolute;writing-mode:vertical-lr;display:flex;flex-wrap:wrap;padding:5px;max-height:80px;box-sizing:border-box'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 70, 80]),
                ("a", [5, 5, 20, 60]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 20]),
                ("d", [45, 5, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:300px;height:300px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap-reverse;padding:5px;max-height:80px;box-sizing:border-box;justify-content:flex-end'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 70, 80]),
                ("a", [5, 15, 20, 60]),
                ("b", [25, 25, 20, 30]),
                ("w", [25, 55, 20, 20]),
                ("d", [45, 15, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:300px;height:300px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;padding:5px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 30, 180]),
                ("a", [5, 5, 20, 60]),
                ("b", [5, 65, 20, 30]),
                ("w", [5, 95, 20, 20]),
                ("d", [5, 115, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:300px;height:300px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;padding:5px;max-height:400px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 30, 180]),
                ("a", [5, 5, 20, 60]),
                ("b", [5, 65, 20, 30]),
                ("w", [5, 95, 20, 20]),
                ("d", [5, 115, 20, 60]),
            ],
        ),
        (
            "<div style='width:300px;height:300px'><div id=c style='float:left;writing-mode:vertical-rl;display:flex;padding:5px;max-height:80px;box-sizing:border-box'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 30, 80]),
                ("a", [5, 5, 20, 25]),
                ("b", [5, 30, 20, 12]),
                ("w", [5, 42, 20, 8]),
                ("d", [5, 50, 20, 25]),
            ],
        ),
    ]);
}

#[test]
fn a_wrapping_row_measured_for_its_content_puts_its_items_on_one_line() {
    // Items 30px long. A flex item, or the content of a float, is measured with
    // its items on one line: 90px, 100px with padding. The float then takes
    // the `max-width` of 80px, while a flex item starts from 90px or 100px and
    // shrinks with its sibling. A vertical row measured for a column flex
    // container is 90px tall and takes its `max-height` of 50px alike, whether
    // or not it has a `float`, which a flex item ignores. A percentage
    // `max-height` of an absolutely positioned row resolves against its
    // containing block, and a border-box `max-width` below the padding leaves
    // the box as wide as the padding.
    assert_rows(&[
        (
            "<div style='position:relative;width:200px;height:200px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:25%'><p id=a style='margin:0;height:30px;width:10px'></p><p id=b style='margin:0;height:30px;width:10px'></p><p id=d style='margin:0;height:30px;width:10px'></p></div></div>",
            &[
                ("c", [0, 0, 30, 50]),
                ("a", [20, 0, 10, 30]),
                ("b", [10, 0, 10, 30]),
                ("d", [0, 0, 10, 30]),
            ],
        ),
        (
            "<div style='position:relative;width:200px;height:200px'><div id=c style='position:absolute;display:flex;flex-wrap:wrap;box-sizing:border-box;padding:0 30px;max-width:40px'><p id=a style='margin:0;width:30px;height:10px'></p><p id=b style='margin:0;width:30px;height:10px'></p><p id=d style='margin:0;width:30px;height:10px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 30]),
                ("a", [30, 0, 0, 10]),
                ("b", [30, 10, 0, 10]),
                ("d", [30, 20, 0, 10]),
            ],
        ),
        (
            "<div style='display:flex;flex-direction:column;width:200px;height:200px'><div id=c style='float:left;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:50px'><p id=a style='margin:0;height:30px;width:10px'></p><p id=b style='margin:0;height:30px;width:10px'></p><p id=d style='margin:0;height:30px;width:10px'></p></div></div>",
            &[
                ("c", [0, 0, 200, 50]),
                ("a", [190, 0, 10, 30]),
                ("b", [123, 0, 10, 30]),
                ("d", [57, 0, 10, 30]),
            ],
        ),
        (
            "<div style='display:flex;width:150px'><div id=c style='display:flex;flex-wrap:wrap;max-width:80px'><p id=a style='margin:0;width:30px;height:10px'></p><p id=b style='margin:0;width:30px;height:10px'></p><p id=d style='margin:0;width:30px;height:10px'></p></div><div id=s style='width:100px;height:10px'></div></div>",
            &[
                ("c", [0, 0, 71, 20]),
                ("a", [0, 0, 30, 10]),
                ("b", [30, 0, 30, 10]),
                ("d", [0, 10, 30, 10]),
                ("s", [71, 0, 79, 10]),
            ],
        ),
        (
            "<div style='display:flex;width:150px'><div id=c style='display:flex;flex-wrap:wrap;max-width:80px;padding:0 5px'><p id=a style='margin:0;width:30px;height:10px'></p><p id=b style='margin:0;width:30px;height:10px'></p><p id=d style='margin:0;width:30px;height:10px'></p></div><div id=s style='width:100px;height:10px'></div></div>",
            &[
                ("c", [0, 0, 76, 20]),
                ("a", [5, 0, 30, 10]),
                ("b", [35, 0, 30, 10]),
                ("d", [5, 10, 30, 10]),
                ("s", [76, 0, 74, 10]),
            ],
        ),
        (
            "<div style='width:300px'><div style='float:left' id=f><div id=c style='display:flex;flex-wrap:wrap;max-width:80px;padding:0 5px;box-sizing:border-box'><p id=a style='margin:0;width:30px;height:10px'></p><p id=b style='margin:0;width:30px;height:10px'></p><p id=d style='margin:0;width:30px;height:10px'></p></div></div></div>",
            &[
                ("f", [0, 0, 80, 20]),
                ("c", [0, 0, 80, 20]),
                ("a", [5, 0, 30, 10]),
                ("b", [35, 0, 30, 10]),
                ("d", [5, 10, 30, 10]),
            ],
        ),
    ]);
}
