//! A flex container whose height size containment settles, and whose main
//! axis runs along that height, flexes its items within the main size their
//! content gives it, within its `min-height` and `max-height`, as if it had
//! no containment. Its items then overflow the height it takes rather than
//! shrink into it. A height a parent stretches or flexes it to is the main size
//! they flex in. Where each box falls is what Chromium lays out for the same
//! documents.

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
fn a_size_contained_flex_container_flexes_its_items_in_their_own_main_size() {
    // Items 60px and 30px tall, or 10px tall and growing and 20px tall, in a
    // container that `contain: size` makes 50px tall (or 0px, or 150px): a
    // column, a wrapping column, a vertical row, positioned, floating, inline,
    // a grid item or a flex item. Its `max-height` still shrinks the items,
    // and a parent that stretches or flexes it gives the items that room.
    assert_rows(&[
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;flex-wrap:wrap'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 0]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:10px;width:20px;flex-grow:1'></p><p id=b style='margin:0;height:20px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 10]),
                ("b", [0, 10, 20, 20]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;justify-content:flex-end'><p id=a style='margin:0;height:10px;width:20px;flex-grow:1'></p><p id=b style='margin:0;height:20px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 10]),
                ("b", [0, 10, 20, 20]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex'><p id=a style='margin:0;width:20px'></p><p id=b style='margin:0;width:20px;height:30px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 50]),
                ("b", [20, 0, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:grid'><p id=a style='margin:0;width:20px'></p><p id=b style='margin:0;width:20px;height:30px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 10]),
                ("b", [0, 10, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;min-height:120px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 120]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;max-height:40px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 40]),
                ("a", [0, 0, 20, 27]),
                ("b", [0, 27, 20, 13]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 33]),
                ("b", [0, 33, 20, 17]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;padding:5px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 60]),
                ("a", [5, 5, 20, 60]),
                ("b", [5, 65, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:150px;display:flex;flex-direction:column'><p id=a style='margin:0;height:10px;width:20px;flex-grow:1'></p><p id=b style='margin:0;height:20px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 150]),
                ("a", [0, 0, 20, 10]),
                ("b", [0, 10, 20, 20]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;align-content:center;flex-wrap:wrap'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [290, 0, 20, 60]),
                ("b", [290, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:150px;display:grid;grid-template-rows:1fr 1fr'><p id=a style='margin:0;width:20px'></p><p id=b style='margin:0;width:20px;height:30px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 150]),
                ("a", [0, 0, 20, 75]),
                ("b", [0, 75, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;'><p id=a style='margin:0;height:60px;width:20px;flex-shrink:0'></p><p id=b style='margin:0;height:30px;width:20px;flex-basis:0;flex-grow:1'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 0]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 0, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='display:grid;width:600px;height:300px;align-items:start'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='display:flex;width:600px;height:300px;align-items:start'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 0, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='display:flex;width:600px;height:300px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 0, 300]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:strict;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:inline-flex;flex-direction:column;width:40px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 40, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='writing-mode:vertical-rl;contain:size;contain-intrinsic-height:50px;contain-intrinsic-width:40px;display:flex'><p id=a style='margin:0;width:60px;height:20px'></p><p id=b style='margin:0;width:30px;height:20px'></p></div></div>",
            &[
                ("c", [0, 0, 40, 50]),
                ("a", [-20, 0, 60, 20]),
                ("b", [10, 20, 30, 20]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:50%;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 0]),
                ("b", [0, 0, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px;height:200px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:50%;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 0]),
                ("b", [0, 0, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div><p id=n style='margin:0;height:10px'></p></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("n", [0, 50, 600, 10]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='float:left;contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 0, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column-reverse'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 30, 20, 60]),
                ("b", [0, 0, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='contain:inline-size;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 90]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='display:flex;width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div><div id=s style='width:10px;height:80px'></div></div>",
            &[
                ("c", [0, 0, 0, 80]),
                ("a", [0, 0, 20, 53]),
                ("b", [0, 53, 20, 27]),
                ("s", [0, 0, 10, 80]),
            ],
        ),
        (
            "<div style='display:flex;width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div><div id=s style='width:10px;height:20px'></div></div>",
            &[
                ("c", [0, 0, 0, 50]),
                ("a", [0, 0, 20, 33]),
                ("b", [0, 33, 20, 17]),
                ("s", [0, 0, 10, 20]),
            ],
        ),
        (
            "<div style='display:grid;width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div><div id=s style='width:10px;height:80px;grid-row:1;grid-column:2'></div></div>",
            &[
                ("c", [0, 0, 295, 80]),
                ("a", [0, 0, 20, 53]),
                ("b", [0, 53, 20, 27]),
                ("s", [295, 0, 10, 80]),
            ],
        ),
        (
            "<div style='display:flex;flex-direction:column;width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 33]),
                ("b", [0, 33, 20, 17]),
            ],
        ),
        (
            "<div style='display:flex;flex-direction:column;width:600px;height:200px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;flex-grow:1'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 200]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
    ]);
}

#[test]
fn a_size_contained_flex_container_places_its_positioned_boxes_in_its_own_height() {
    // The same items in a column container that containment makes 50px tall,
    // or a `max-height` 40px: a box it positions at its bottom, halfway down
    // or at its start is placed in that height, inside its padding, whatever
    // the height its items overflow to. A positioned container with a column
    // flex parent is no item of it and keeps its items' heights.
    assert_rows(&[
        (
            "<div style='width:600px'><div id=c style='position:relative;contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=o style='position:absolute;margin:0;bottom:0;height:10px;width:10px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("o", [0, 40, 10, 10]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='position:relative;contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=o style='position:absolute;margin:0;top:50%;height:10px;width:10px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("o", [0, 25, 10, 10]),
            ],
        ),
        (
            "<div style='width:600px;display:flex;flex-direction:column;position:relative;height:300px'><div id=c style='position:absolute;contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 0, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='position:relative;contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=o style='position:absolute;margin:0;height:10px;width:10px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 50]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 30]),
                ("o", [0, 0, 10, 10]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='position:relative;contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;padding:5px;border:2px solid'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=o style='position:absolute;margin:0;bottom:0;height:10px;width:10px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 64]),
                ("a", [7, 7, 20, 60]),
                ("b", [7, 67, 20, 30]),
                ("o", [7, 52, 10, 10]),
            ],
        ),
        (
            "<div style='width:600px'><div id=c style='position:relative;contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column;max-height:40px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p><p id=o style='position:absolute;margin:0;bottom:0;height:10px;width:10px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 40]),
                ("a", [0, 0, 20, 27]),
                ("b", [0, 27, 20, 13]),
                ("o", [0, 30, 10, 10]),
            ],
        ),
    ]);
}
