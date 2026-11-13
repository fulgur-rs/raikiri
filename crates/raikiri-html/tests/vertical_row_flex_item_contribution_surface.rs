//! A row flex container in a vertical writing mode lays its items out along
//! its height, its inline size. The intrinsic heights of such a container are
//! those of its items: an item whose flex base size is a length contributes
//! at least that size when it cannot shrink and at most that size when it
//! cannot grow, within its own limits (CSS Flexbox 1 section 9.9.1), and the
//! min-content height of a container that wraps its items is that of its
//! tallest item. Where each box falls is what Chromium lays out for the same
//! documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A document body and the id and border box of each box with an id in it.
type Row = (&'static str, &'static [(&'static str, [i32; 4])]);

/// The id and border box of every box with an id in `body`, laid out on one
/// page 800px wide and 600px tall in 10px Ahem, with border-box `div`s.
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
        "<!doctype html><style>@page{{size:800px 600px;margin:0}} body{{margin:0;font:10px/10px Ahem}} div{{box-sizing:border-box}}</style><body>{body}</body>"
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
fn a_vertical_wrapping_row_is_as_tall_as_its_items_on_one_line() {
    // Items whose flex base sizes are lengths, in a block-level row that
    // wraps them in a vertical writing mode: an item that cannot shrink
    // contributes at least its flex base size to the max-content height, one
    // that cannot grow at most that size, within its own limits and with its
    // margins, whatever its content, and a scroll container contributes its
    // flex base size where it cannot grow.
    assert_rows(&[
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>xxxx</div><div id=i style='flex:1 0 70px'>xx</div></div>",
            &[
                ("o", [0, 0, 10, 110]),
                ("m", [0, 0, 10, 40]),
                ("i", [0, 40, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:0 1 70px'>x</div></div>",
            &[
                ("o", [0, 0, 20, 70]),
                ("m", [10, 0, 10, 60]),
                ("i", [0, 0, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:1 1 20px'>xxx</div></div>",
            &[
                ("o", [0, 0, 10, 90]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 30]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;min-height:50px'>x</div><div id=i style='flex:1 0 20px;max-height:10px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 50]),
                ("i", [0, 50, 10, 10]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;height:10px'>x</div><div id=i style='flex:1 0 20px;height:50px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:0 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 30]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;margin-top:5px'>x</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 55]),
                ("m", [0, 5, 10, 30]),
                ("i", [0, 35, 10, 20]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;height:min-content'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content'><div id=m style='flex:1 1 30px'>xxxx</div><div id=i style='flex:1 1 70px'>xx</div></div>",
            &[
                ("o", [0, 0, 20, 60]),
                ("m", [10, 0, 10, 60]),
                ("i", [0, 0, 10, 60]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content'><div id=m style='flex:0 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 30]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content'><div id=m style='flex:1 0 30px;max-height:20px'>xxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 40]),
                ("m", [0, 0, 10, 20]),
                ("i", [0, 20, 10, 20]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap;height:max-content'><div id=m style='flex:1 1 30px;min-height:50px'>x</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 70]),
                ("m", [0, 0, 10, 50]),
                ("i", [0, 50, 10, 20]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_inline_wrapping_row_is_as_tall_as_its_items_on_one_line() {
    // The same items in an inline-level row of `vertical-lr`, which fits its
    // height to its content.
    assert_rows(&[
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>xxxx</div><div id=i style='flex:1 0 70px'>xx</div></div>",
            &[
                ("o", [0, 0, 10, 110]),
                ("m", [0, 0, 10, 40]),
                ("i", [0, 40, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:0 1 70px'>x</div></div>",
            &[
                ("o", [0, 0, 20, 70]),
                ("m", [0, 0, 10, 60]),
                ("i", [10, 0, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:1 1 20px'>xxx</div></div>",
            &[
                ("o", [0, 0, 10, 90]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 30]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:1 0 30px;min-height:50px'>x</div><div id=i style='flex:1 0 20px;max-height:10px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 50]),
                ("i", [0, 50, 10, 10]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:1 0 30px;height:10px'>x</div><div id=i style='flex:1 0 20px;height:50px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:1 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:0 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 30]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:1 0 30px;margin-top:5px'>x</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 55]),
                ("m", [0, 5, 10, 30]),
                ("i", [0, 35, 10, 20]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:1 0 30px;height:min-content'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_floating_wrapping_row_is_as_tall_as_its_items_on_one_line() {
    // The same items in a floating row.
    assert_rows(&[
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>xxxx</div><div id=i style='flex:1 0 70px'>xx</div></div>",
            &[
                ("o", [0, 0, 10, 110]),
                ("m", [0, 0, 10, 40]),
                ("i", [0, 40, 10, 70]),
            ],
        ),
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:0 1 70px'>x</div></div>",
            &[
                ("o", [0, 0, 20, 70]),
                ("m", [10, 0, 10, 60]),
                ("i", [0, 0, 10, 70]),
            ],
        ),
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:1 1 20px'>xxx</div></div>",
            &[
                ("o", [0, 0, 10, 90]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 30]),
            ],
        ),
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;min-height:50px'>x</div><div id=i style='flex:1 0 20px;max-height:10px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 50]),
                ("i", [0, 50, 10, 10]),
            ],
        ),
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;height:10px'>x</div><div id=i style='flex:1 0 20px;height:50px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:0 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 30]),
            ],
        ),
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;margin-top:5px'>x</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 55]),
                ("m", [0, 5, 10, 30]),
                ("i", [0, 35, 10, 20]),
            ],
        ),
        (
            "<div id=o style='float:left;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;height:min-content'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_positioned_wrapping_row_is_as_tall_as_its_items_on_one_line() {
    // The same items in an absolutely positioned row.
    assert_rows(&[
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>xxxx</div><div id=i style='flex:1 0 70px'>xx</div></div>",
            &[
                ("o", [0, 0, 10, 110]),
                ("m", [0, 0, 10, 40]),
                ("i", [0, 40, 10, 70]),
            ],
        ),
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:0 1 70px'>x</div></div>",
            &[
                ("o", [0, 0, 20, 70]),
                ("m", [10, 0, 10, 60]),
                ("i", [0, 0, 10, 70]),
            ],
        ),
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:1 1 20px'>xxx</div></div>",
            &[
                ("o", [0, 0, 10, 90]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 30]),
            ],
        ),
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;min-height:50px'>x</div><div id=i style='flex:1 0 20px;max-height:10px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 50]),
                ("i", [0, 50, 10, 10]),
            ],
        ),
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;height:10px'>x</div><div id=i style='flex:1 0 20px;height:50px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:0 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 30]),
            ],
        ),
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;margin-top:5px'>x</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 55]),
                ("m", [0, 5, 10, 30]),
                ("i", [0, 35, 10, 20]),
            ],
        ),
        (
            "<div id=o style='position:absolute;display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;height:min-content'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_single_line_row_sums_its_items_contributions() {
    // The same items in a row of one line, whose items flex into the height
    // their contributions add up to.
    assert_rows(&[
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:1 0 30px'>xxxx</div><div id=i style='flex:1 0 70px'>xx</div></div>",
            &[
                ("o", [0, 0, 10, 110]),
                ("m", [0, 0, 10, 40]),
                ("i", [0, 40, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:0 1 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 70]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 10]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:0 1 30px'>xxxxxx</div><div id=i style='flex:1 1 20px'>xxx</div></div>",
            &[
                ("o", [0, 0, 10, 90]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 30]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:1 0 30px;min-height:50px'>x</div><div id=i style='flex:1 0 20px;max-height:10px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 60]),
                ("m", [0, 0, 10, 50]),
                ("i", [0, 50, 10, 10]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:1 0 30px;height:10px'>x</div><div id=i style='flex:1 0 20px;height:50px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:1 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 45]),
                ("i", [0, 45, 10, 35]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:0 0 30px;overflow:hidden'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 50]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 20]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:1 0 30px;margin-top:5px'>x</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 55]),
                ("m", [0, 5, 10, 30]),
                ("i", [0, 35, 10, 20]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl'><div id=m style='flex:1 0 30px;height:min-content'>xxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 60]),
                ("i", [0, 60, 10, 20]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_wrapping_row_fits_the_room_it_is_offered() {
    // A wrapping row offered less height than its items on one line take is
    // no shorter than its tallest item, and one in a flex container, at a
    // definite width, with padding and border, a gap, margins or an item of
    // the other writing mode sums its items as above.
    assert_rows(&[
        (
            "<div style='height:50px'><div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>xxxxxxxx</div><div id=i style='flex:1 0 70px'>x</div></div></div>",
            &[
                ("o", [0, 0, 20, 80]),
                ("m", [10, 0, 10, 80]),
                ("i", [0, 0, 10, 80]),
            ],
        ),
        (
            "<div style='height:50px'><div id=o style='display:inline-flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>xxxxxxxx</div></div></div>",
            &[
                ("o", [0, 0, 20, 80]),
                ("m", [10, 0, 10, 80]),
                ("i", [0, 0, 10, 80]),
            ],
        ),
        (
            "<div style='height:50px'><div id=o style='float:left;display:flex;writing-mode:vertical-lr;flex-wrap:wrap'><div id=m style='flex:0 0 30px;overflow:hidden'>xxxxxxxx</div><div id=i style='flex:1 0 20px'>x</div></div></div>",
            &[
                ("o", [0, 0, 10, 80]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 50]),
            ],
        ),
        (
            "<div style='display:flex'><div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div></div>",
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            "<div style='display:flex;flex-direction:column;align-items:start'><div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div></div>",
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap;width:30px'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px;align-self:stretch'>x</div></div>",
            &[
                ("o", [0, 0, 30, 100]),
                ("m", [0, 0, 30, 30]),
                ("i", [0, 30, 30, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap;padding:5px;border:2px solid'><div id=m style='flex:1 0 30px;margin-top:4px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 24, 118]),
                ("m", [7, 11, 10, 30]),
                ("i", [7, 41, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap;max-height:80px'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 20, 80]),
                ("m", [10, 0, 10, 80]),
                ("i", [0, 0, 10, 80]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap;gap:5px'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 105]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 35, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;box-sizing:content-box;padding-top:5px'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 105]),
                ("m", [0, 0, 10, 35]),
                ("i", [0, 35, 10, 70]),
            ],
        ),
        (
            "<div id=o style='display:flex;writing-mode:vertical-rl;flex-wrap:wrap'><div id=m style='flex:1 0 30px;writing-mode:horizontal-tb'>x</div><div id=i style='flex:1 0 70px'>x</div></div>",
            &[
                ("o", [0, 0, 10, 100]),
                ("m", [0, 0, 10, 30]),
                ("i", [0, 30, 10, 70]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_inline_row_measures_its_items_at_its_own_width() {
    // An inline-level row whose width, its cross size, is stated measures its
    // items at that width less its insets, and its padding and border stay
    // whole however low its `max-height` is.
    assert_rows(&[
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-rl;flex-wrap:wrap;width:30px'><div id=m style='flex:1 0 30px'>xx xx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 30, 70]),
                ("m", [0, 0, 30, 40]),
                ("i", [0, 40, 30, 30]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-rl;width:30px'><div id=m style='flex:1 0 30px'>xxxx xxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 30, 110]),
                ("m", [0, 0, 30, 60]),
                ("i", [0, 60, 30, 50]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-rl;flex-wrap:wrap;width:30px'><div id=m style='flex:1 1 30px;width:50%'>xxxx xxxx</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 30, 110]),
                ("m", [15, 0, 15, 60]),
                ("i", [0, 60, 30, 50]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-rl;flex-wrap:wrap;max-height:5px;padding:10px 0'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 20px'>x</div></div>",
            &[
                ("o", [0, 0, 20, 20]),
                ("m", [10, 10, 10, 30]),
                ("i", [0, 10, 10, 20]),
            ],
        ),
        (
            "<div id=o style='display:inline-flex;writing-mode:vertical-rl;flex-wrap:wrap;padding:5px;border:3px solid'><div id=m style='flex:1 0 30px'>x</div><div id=i style='flex:1 0 20px'>xxxxx</div></div>",
            &[
                ("o", [0, 0, 26, 96]),
                ("m", [8, 8, 10, 30]),
                ("i", [8, 38, 10, 50]),
            ],
        ),
    ]);
}
