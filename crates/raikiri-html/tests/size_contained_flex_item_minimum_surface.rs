//! A flex item with size containment along the main axis of its container has
//! no content-based automatic minimum size there: it shrinks below the size
//! containment gives it (its `contain-intrinsic-size`, or the size the ratio
//! gives it), down to its padding and border, unless it states a minimum or
//! does not shrink. A grid item keeps its contained size as its content-based
//! minimum. Where each box falls is what Chromium lays out for the same
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
fn a_size_contained_column_item_shrinks_below_its_contained_height() {
    // Items that `contain: size` makes 50px (or 30px) tall in a column 40px
    // tall: a flex container, a block, a grid, one that clips, states a
    // height, padding, or shares the column. A `min-height`, `flex-shrink: 0`
    // or a containment across the main axis keep the item's size.
    assert_rows(&[
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 27]),
                ("b", [0, 327, 20, 13]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:grid'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:30px;height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;min-height:45px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 45]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;flex-shrink:0'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 50]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div><div id=c style='contain:size;contain-intrinsic-height:30px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 25]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
                ("c", [0, 325, 600, 15]),
                ("a", [0, 325, 20, 60]),
                ("b", [0, 385, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [20, 300, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;padding:10px 0 15px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 310, 20, 10]),
                ("b", [0, 320, 20, 5]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;box-sizing:border-box;padding:10px 0 15px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 310, 20, 10]),
                ("b", [0, 320, 20, 5]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div><div id=d style='height:10px'></div></div>",
            &[
                ("c", [0, 300, 600, 33]),
                ("a", [0, 300, 20, 22]),
                ("b", [0, 322, 20, 11]),
                ("d", [0, 333, 600, 7]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;min-height:auto;flex-basis:0;flex-grow:1'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;max-height:30px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 30]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px;flex-wrap:wrap'><div id=c style='contain:size;contain-intrinsic-height:50px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
    ]);
}

#[test]
fn a_size_contained_row_item_shrinks_below_its_contained_width() {
    // Items that `contain: size` makes 700px wide (or 800px through their
    // flex basis) in a row 600px wide, with or without padding.
    assert_rows(&[
        (
            "<div style='margin-top:300px;display:flex;flex-direction:row;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-width:700px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 27]),
                ("b", [0, 327, 20, 13]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:row;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-width:700px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-width:700px;padding:0 30px;display:flex'><p id=a style='margin:0;height:10px;width:500px'></p><p id=b style='margin:0;height:10px;width:300px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [30, 300, 338, 10]),
                ("b", [368, 300, 203, 10]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-width:700px;width:200px;flex-basis:800px'><p id=a style='margin:0;height:10px;width:500px'></p><p id=b style='margin:0;height:10px;width:300px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 500, 10]),
                ("b", [0, 310, 300, 10]),
            ],
        ),
    ]);
}

#[test]
fn a_size_contained_ratio_item_shrinks_and_a_grid_item_does_not() {
    // Items with an aspect ratio that `contain: size` sizes past the room of
    // their column or row shrink into it, also when they state the size across.
    // A grid item that `contain: size` makes 700px wide keeps that width as
    // its minimum in an `auto` track that does not cap it.
    assert_rows(&[
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;aspect-ratio:12'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;aspect-ratio:6;width:300px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 300, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;aspect-ratio:6;width:300px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 300, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-width:700px;aspect-ratio:10'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 400, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-width:700px;aspect-ratio:10;height:70px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 70]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;aspect-ratio:12;overflow:hidden'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 60]),
                ("b", [0, 360, 20, 30]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:grid;grid-template-rows:minmax(auto,40px);width:600px;height:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;display:flex;flex-direction:column'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:30px;width:20px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 40]),
                ("a", [0, 300, 20, 27]),
                ("b", [0, 327, 20, 13]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:grid;grid-template-columns:minmax(auto,600px);width:600px'><div id=c style='contain:size;contain-intrinsic-width:700px;display:flex'><p id=a style='margin:0;height:10px;width:500px'></p><p id=b style='margin:0;height:10px;width:300px'></p></div></div>",
            &[
                ("c", [0, 300, 600, 0]),
                ("a", [0, 300, 375, 10]),
                ("b", [375, 300, 225, 10]),
            ],
        ),
        (
            "<div style='margin-top:300px;display:grid;grid-template-columns:fit-content(100px);width:600px'><div id=c style='contain:size;contain-intrinsic-width:700px;display:flex'><p id=a style='margin:0;height:10px;width:500px'></p><p id=b style='margin:0;height:10px;width:300px'></p></div></div>",
            &[
                ("c", [0, 300, 700, 0]),
                ("a", [0, 300, 438, 10]),
                ("b", [438, 300, 263, 10]),
            ],
        ),
    ]);
}

#[test]
fn a_size_contained_item_of_a_vertical_container_shrinks_along_its_main_axis() {
    // Items that `contain: size` makes 50px tall in a `vertical-rl` row 40px
    // tall, and 60px wide in a `vertical-rl` column 40px wide; a `vertical-rl`
    // item in a horizontal column 40px tall.
    assert_rows(&[
        (
            "<div style='margin-top:300px;writing-mode:vertical-rl;display:flex;height:40px;width:600px'><div id=c style='contain:size;contain-intrinsic-height:50px;contain-intrinsic-width:30px'><p id=a style='margin:0;height:10px;width:20px'></p></div></div>",
            &[("c", [0, 300, 600, 40]), ("a", [580, 300, 20, 10])],
        ),
        (
            "<div style='margin-top:300px;writing-mode:vertical-rl;display:flex;flex-direction:column;height:300px;width:40px'><div id=c style='contain:size;contain-intrinsic-height:50px;contain-intrinsic-width:60px'><p id=a style='margin:0;height:10px;width:20px'></p></div></div>",
            &[("c", [0, 300, 40, 300]), ("a", [20, 300, 20, 10])],
        ),
        (
            "<div style='margin-top:300px;display:flex;flex-direction:column;height:40px;width:600px'><div id=c style='writing-mode:vertical-rl;contain:size;contain-intrinsic-height:50px;contain-intrinsic-width:30px'><p id=a style='margin:0;height:10px;width:20px'></p></div></div>",
            &[("c", [0, 300, 600, 40]), ("a", [580, 300, 20, 10])],
        ),
    ]);
}
