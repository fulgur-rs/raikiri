//! An absolutely positioned flex container in a vertical writing mode whose
//! content decides its height settles that height, its inline size, before
//! its width. Percentage heights of its items resolve against the height, and
//! its lines break against it, as Chromium lays out the same documents.

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
fn a_vertical_positioned_flex_container_resolves_percentage_heights_against_its_content_height() {
    // Items 60px, 50%, 20px and 60px tall in a container that `max-height`
    // stops at 80px: the 50% item is 40px, and the items make three lines.
    // With no `max-height`, the height is that of the items on one line with
    // the percentage read as `auto` (140px), and the 50% item is 70px. The
    // same holds in vertical-lr, without wrapping, with a `min-height`, a
    // padding and a border, a percentage `max-height`, a stated or inset
    // width, inline-flex placed from the bottom, a container in a vertical
    // containing block, and a `fit-content` height, and in a column
    // container whose lines run across the height. A block container, and a
    // container whose `top` and `bottom` fix its height, resolve it as well,
    // while a horizontal column container, whose height is its block size,
    // leaves it `auto`.
    assert_rows(&[
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 80]),
                ("a", [40, 0, 20, 60]),
                ("b", [20, 0, 20, 40]),
                ("w", [20, 40, 20, 20]),
                ("d", [0, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 40, 140]),
                ("a", [20, 0, 20, 60]),
                ("b", [20, 60, 20, 70]),
                ("w", [0, 0, 20, 20]),
                ("d", [0, 20, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-lr;display:flex;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 80]),
                ("a", [0, 0, 20, 60]),
                ("b", [20, 0, 20, 40]),
                ("w", [20, 40, 20, 20]),
                ("d", [40, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 20, 140]),
                ("a", [0, 0, 20, 40]),
                ("b", [0, 40, 20, 47]),
                ("w", [0, 87, 20, 13]),
                ("d", [0, 100, 20, 40]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;min-height:200px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 40, 200]),
                ("a", [20, 0, 20, 60]),
                ("b", [20, 60, 20, 100]),
                ("w", [20, 160, 20, 20]),
                ("d", [0, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px;padding:5px 3px;border:2px solid'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 70, 94]),
                ("a", [45, 7, 20, 60]),
                ("b", [25, 7, 20, 40]),
                ("w", [25, 47, 20, 20]),
                ("d", [5, 7, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 80, 60]),
                ("a", [60, 0, 20, 60]),
                ("b", [40, 0, 20, 30]),
                ("w", [20, 0, 20, 20]),
                ("d", [0, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:inline-flex;flex-wrap:wrap;max-height:80px;left:10px;bottom:20px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [10, 300, 60, 80]),
                ("a", [50, 300, 20, 60]),
                ("b", [30, 300, 20, 40]),
                ("w", [30, 340, 20, 20]),
                ("d", [10, 300, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:50%'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 40, 140]),
                ("a", [20, 0, 20, 60]),
                ("b", [20, 60, 20, 70]),
                ("w", [0, 0, 20, 20]),
                ("d", [0, 20, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px;width:30px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 30, 80]),
                ("a", [10, 0, 20, 60]),
                ("b", [-10, 0, 20, 40]),
                ("w", [-10, 40, 20, 20]),
                ("d", [-30, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px;left:0;right:0'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 600, 80]),
                ("a", [580, 0, 20, 60]),
                ("b", [380, 0, 20, 40]),
                ("w", [380, 40, 20, 20]),
                ("d", [180, 0, 20, 60]),
            ],
        ),
        (
            "<div style='writing-mode:vertical-rl;position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [540, 0, 60, 80]),
                ("a", [580, 0, 20, 60]),
                ("b", [560, 0, 20, 40]),
                ("w", [560, 40, 20, 20]),
                ("d", [540, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:80px;height:fit-content'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 80]),
                ("a", [40, 0, 20, 60]),
                ("b", [20, 0, 20, 40]),
                ("w", [20, 40, 20, 20]),
                ("d", [0, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:block'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 80, 60]),
                ("a", [60, 0, 20, 60]),
                ("b", [40, 0, 20, 30]),
                ("w", [20, 0, 20, 20]),
                ("d", [0, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:block;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 80, 60]),
                ("a", [60, 0, 20, 60]),
                ("b", [40, 0, 20, 30]),
                ("w", [20, 0, 20, 20]),
                ("d", [0, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;writing-mode:vertical-rl;display:flex;flex-wrap:wrap;top:0;bottom:300px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 60, 100]),
                ("a", [40, 0, 20, 60]),
                ("b", [20, 0, 20, 50]),
                ("w", [20, 50, 20, 20]),
                ("d", [0, 0, 20, 60]),
            ],
        ),
        (
            "<div style='position:relative;width:600px;height:400px'><div id=c style='position:absolute;display:flex;flex-direction:column;flex-wrap:wrap;max-height:80px'><p id=a style='margin:0;height:60px;width:20px'></p><p id=b style='margin:0;height:50%;width:20px'></p><p id=w style='margin:0;height:20px;width:20px'></p><p id=d style='margin:0;height:60px;width:20px'></p></div></div>",
            &[
                ("c", [0, 0, 40, 80]),
                ("a", [0, 0, 20, 60]),
                ("b", [0, 60, 20, 0]),
                ("w", [0, 60, 20, 20]),
                ("d", [20, 0, 20, 60]),
            ],
        ),
    ]);
}
