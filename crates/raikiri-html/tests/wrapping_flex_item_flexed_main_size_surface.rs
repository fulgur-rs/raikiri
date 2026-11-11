//! A flex item that wraps its own items onto lines along the main axis it
//! shares with its container breaks those lines against the size the
//! container flexes it to, even where that size is not definite because the
//! container's own size is not. It keeps that size, and percentages of its
//! items still find no definite size to refer to. Where each box falls is
//! what Chromium lays out for the same documents.

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
fn a_wrapping_column_item_breaks_its_lines_against_the_height_its_container_flexes_it_to() {
    // Items of 90px, 30px, 30px and 90px in a column item that its column
    // container shrinks to a `max-height` of 100px make three lines; grown
    // to 400px, or to its own `max-height`, they make one or two. The
    // container states a height, a `min-height` or a `max-height`, or wraps
    // its own items; the item has padding (a percentage one too), a border,
    // gaps, a distribution of its items or lines, lines in reverse, a growing
    // item, or items in reverse order; the container floats or is inline,
    // and is then as wide as the item's items on one line. A percentage
    // height of an item resolves only against a container of a stated
    // height.
    assert_rows(&[
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:25%;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 0]),
                ("w", [20, 0, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:25%;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 25]),
                ("w", [20, 25, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;justify-content:center'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 5, 20, 90]),
                ("b", [20, 20, 20, 30]),
                ("w", [20, 50, 20, 30]),
                ("d", [40, 5, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;min-height:400px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;flex-grow:1'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 400]),
                ("c", [0, 0, 40, 400]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [0, 150, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;min-height:400px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;flex-grow:1'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:25%;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 400]),
                ("c", [0, 0, 40, 400]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 0]),
                ("w", [0, 90, 20, 30]),
                ("d", [0, 120, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;min-height:400px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;flex-grow:1;max-height:150px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 400]),
                ("c", [0, 0, 40, 150]),
                ("a", [0, 0, 20, 90]),
                ("b", [0, 90, 20, 30]),
                ("w", [0, 120, 20, 30]),
                ("d", [20, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;flex-basis:200px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px;flex-wrap:wrap'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;max-height:200px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:row;height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:row;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;padding:5px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 50, 100]),
                ("a", [5, 5, 20, 90]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 30]),
                ("d", [45, 5, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;padding:5px;box-sizing:border-box'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [5, 5, 20, 90]),
                ("b", [25, 5, 20, 30]),
                ("w", [25, 35, 20, 30]),
                ("d", [45, 5, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;row-gap:10px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 40, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;flex-wrap:wrap-reverse'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [20, 0, 20, 90]),
                ("b", [0, 0, 20, 30]),
                ("w", [0, 30, 20, 30]),
                ("d", [-20, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;align-content:space-between;width:100px'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 100, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [40, 0, 20, 30]),
                ("w", [40, 30, 20, 30]),
                ("d", [80, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p><p id=e style='margin:0;height:20px;width:20px;flex-grow:1'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
                ("e", [60, 0, 20, 100]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;border:3px solid'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 46, 100]),
                ("a", [3, 3, 20, 90]),
                ("b", [23, 3, 20, 30]),
                ("w", [23, 33, 20, 30]),
                ("d", [43, 3, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0;flex-direction:column-reverse'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 10, 20, 90]),
                ("b", [20, 70, 20, 30]),
                ("w", [20, 40, 20, 30]),
                ("d", [40, 10, 20, 90]),
            ],
        ),
        (
            "<div id=p style='float:left;display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:40px;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 40, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:inline-flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 20, 100]),
                ("c", [0, 0, 20, 100]),
                ("a", [0, 0, 20, 90]),
                ("b", [20, 0, 20, 30]),
                ("w", [20, 30, 20, 30]),
                ("d", [40, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px;width:200px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;width:60px;min-height:0;padding-top:5%'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 200, 100]),
                ("c", [0, 0, 60, 100]),
                ("a", [0, 10, 20, 90]),
                ("b", [20, 10, 20, 30]),
                ("w", [20, 40, 20, 30]),
                ("d", [40, 10, 20, 90]),
            ],
        ),
    ]);
}

#[test]
fn a_wrapping_item_breaks_its_lines_against_the_width_or_height_it_is_flexed_to_in_a_vertical_writing_mode()
 {
    // A column item of a column container in a vertical writing mode, whose
    // main axis is the width, and a vertical row item of a horizontal column
    // container, whose main axis is the height. A percentage padding of the
    // column item refers to the height of its vertical container.
    assert_rows(&[
        (
            "<div id=p style='writing-mode:vertical-rl;display:flex;flex-direction:column;max-width:100px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;height:40px;min-width:0'><p id=a style='margin:0;width:90px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:30px;height:20px'></p><p id=d style='margin:0;width:90px;height:20px'></p></div></div>",
            &[
                ("p", [0, 0, 100, 40]),
                ("c", [0, 0, 100, 40]),
                ("a", [10, 0, 90, 20]),
                ("b", [70, 20, 30, 20]),
                ("w", [40, 20, 30, 20]),
                ("d", [10, 40, 90, 20]),
            ],
        ),
        (
            "<div id=p style='display:flex;flex-direction:column;max-height:100px'><div id=c style='display:flex;flex-direction:row;flex-wrap:wrap;writing-mode:vertical-rl;width:40px;min-height:0'><p id=a style='margin:0;height:90px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=w style='margin:0;height:30px;width:20px'></p><p id=d style='margin:0;height:90px;width:20px'></p></div></div>",
            &[
                ("p", [0, 0, 1280, 100]),
                ("c", [0, 0, 40, 100]),
                ("a", [20, 0, 20, 90]),
                ("b", [0, 0, 20, 30]),
                ("w", [0, 30, 20, 30]),
                ("d", [-20, 0, 20, 90]),
            ],
        ),
        (
            "<div id=p style='writing-mode:vertical-rl;display:flex;flex-direction:column;max-width:100px;height:600px'><div id=c style='display:flex;flex-direction:column;flex-wrap:wrap;height:40px;min-width:0;padding-left:5%'><p id=a style='margin:0;width:90px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p><p id=w style='margin:0;width:30px;height:20px'></p><p id=d style='margin:0;width:90px;height:20px'></p></div></div>",
            &[
                ("p", [0, 0, 100, 600]),
                ("c", [0, 0, 100, 40]),
                ("a", [30, 0, 70, 20]),
                ("b", [70, 20, 30, 20]),
                ("w", [40, 20, 30, 20]),
                ("d", [30, 40, 70, 20]),
            ],
        ),
    ]);
}
