//! Percentages along the inline axis of an orthogonal flow whose container
//! leaves that size open resolve against the initial containing block (CSS
//! Writing Modes 4 §7.3.1), and percentage main margins of column flex items
//! resolve against the cross size the container settles (CSS Sizing 3
//! §5.2.1).
//!
//! The expected boxes are those Chromium lays out in an 800x600 viewport, the
//! page area here.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

const HORIZONTAL_COLUMN: &str = "display:inline-flex;flex-direction:column";
const VERTICAL_COLUMN: &str = "display:flex;flex-direction:column;writing-mode:vertical-rl";

/// The border boxes of the container `c` and its child `a`, as
/// `[x, y, width, height]`, for a container styled `container` holding a
/// child styled `item` with the text `text`.
fn boxes(container: &str, item: &str, text: &str) -> [[f32; 4]; 2] {
    let html = format!(
        "<!DOCTYPE html><html><head><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:0;width:800px;font:10px/10px Ahem}}div{{box-sizing:border-box}}\
         </style></head><body><div id=c style=\"{container}\"><div id=a style=\"{item}\">\
         {text}</div></div></body></html>"
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
        panic!("layout did not complete");
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let rect = |id: &str| {
        let fragment = page
            .fragments()
            .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
            .unwrap_or_else(|| panic!("no box for {id}"));
        let r = fragment.rect();
        [r.x, r.y, r.width, r.height]
    };
    [rect("c"), rect("a")]
}

/// Asserts the boxes match Chromium's, which rounds each size and margin to
/// 1/64px, within the 1/32px those roundings add up to.
#[track_caller]
fn assert_boxes(container: &str, item: &str, text: &str, expected: [[f32; 4]; 2]) {
    let actual = boxes(container, item, text);
    let close = actual
        .iter()
        .flatten()
        .zip(expected.iter().flatten())
        .all(|(a, e)| (a - e).abs() <= 1.0 / 32.0);
    assert!(close, "{container} / {item}: {actual:?} != {expected:?}");
}

#[test]
fn percentage_inline_sizes_of_orthogonal_flex_items_refer_to_the_page_area() {
    let h = |item: &str, expected| {
        assert_boxes(
            HORIZONTAL_COLUMN,
            &format!("writing-mode:vertical-rl;{item}"),
            "xxxx xxxx xx",
            expected,
        )
    };
    h(
        "flex:0 0 120px;max-height:80px;min-height:50%",
        [[0., 0., 10., 300.], [0., 0., 10., 300.]],
    );
    h("height:50%", [[0., 0., 10., 300.], [0., 0., 10., 300.]]);
    h(
        "flex:0 0 120px;max-height:10%",
        [[0., 0., 30., 60.], [0., 0., 30., 60.]],
    );
    h(
        "flex:0 0 20%;min-height:30%",
        [[0., 0., 10., 180.], [0., 0., 10., 180.]],
    );
    h(
        "flex:0 0 20px;min-height:calc(10% + 5px)",
        [[0., 0., 10., 65.], [0., 0., 10., 65.]],
    );
    let v = |item: &str, expected| {
        assert_boxes(
            VERTICAL_COLUMN,
            &format!("writing-mode:horizontal-tb;{item}"),
            "xxxx xxxx xxxx",
            expected,
        )
    };
    v(
        "flex:0 0 160px;max-width:80px;min-width:50%",
        [[0., 0., 400., 10.], [0., 0., 400., 10.]],
    );
    v("width:50%", [[0., 0., 400., 10.], [0., 0., 400., 10.]]);
    v(
        "flex:0 0 160px;max-width:10%",
        [[0., 0., 80., 30.], [0., 0., 80., 30.]],
    );
    // In a row container the inline size of such an item is its cross size,
    // which refers to the page area the same way.
    assert_boxes(
        "display:flex",
        "writing-mode:vertical-rl;height:50%",
        "xxxx xxxx xxxx",
        [[0., 0., 800., 300.], [0., 0., 10., 300.]],
    );
    assert_boxes(
        "display:flex;writing-mode:vertical-rl",
        "writing-mode:horizontal-tb;width:50%",
        "xxxx xxxx xxxx",
        [[0., 0., 400., 10.], [0., 0., 400., 10.]],
    );
    // An item in the writing mode of its container keeps resolving against
    // the open main size.
    assert_boxes(
        "display:flex;flex-direction:column",
        "flex:0 0 120px;min-height:50%",
        "xxxx xxxx xx",
        [[0., 0., 800., 120.], [0., 0., 800., 120.]],
    );
}

#[test]
fn a_percentage_height_of_a_vertical_box_in_horizontal_flow_refers_to_the_page_area() {
    let text = "xxxx xxxx xxxx";
    for container in [
        "writing-mode:vertical-rl;height:50%",
        "display:flex;flex-direction:column;writing-mode:vertical-rl;height:50%",
    ] {
        assert_boxes(
            "display:block",
            container,
            text,
            [[0., 0., 800., 300.], [0., 0., 10., 300.]],
        );
    }
    assert_boxes(
        "display:flex;flex-direction:column;writing-mode:vertical-rl;height:50%",
        "writing-mode:horizontal-tb;flex:0 0 160px;max-width:80px;margin:3%",
        text,
        [[0., 0., 98., 300.], [9., 9., 80., 282.]],
    );
    assert_boxes(
        "display:flex;flex-direction:column;writing-mode:vertical-rl;height:50%",
        "writing-mode:horizontal-tb;max-width:80px",
        text,
        [[0., 0., 80., 300.], [0., 0., 80., 300.]],
    );
}

#[test]
fn percentage_main_margins_of_column_items_refer_to_the_settled_cross_size() {
    let h = |container: &str, item: &str, expected| {
        assert_boxes(
            container,
            &format!("writing-mode:vertical-rl;flex:0 0 120px;max-height:80px;{item}"),
            "xxxx xxxx xx",
            expected,
        )
    };
    h(
        HORIZONTAL_COLUMN,
        "padding:5%;margin:3%",
        [[0., 0., 20., 81.1875], [0.59375, 0.59375, 18.8125, 80.]],
    );
    h(
        &format!("{HORIZONTAL_COLUMN};max-height:100px;padding:3px;border:2px solid"),
        "padding:5%;margin:3%",
        [[0., 0., 30., 91.1875], [5.59375, 5.59375, 18.8125, 80.]],
    );
    h(
        HORIZONTAL_COLUMN,
        "margin:3%",
        [[0., 0., 20., 81.1875], [0.59375, 0.59375, 18.8125, 80.]],
    );
    h(
        HORIZONTAL_COLUMN,
        "padding:5%",
        [[0., 0., 20., 80.], [0., 0., 20., 80.]],
    );
    let v = |container: &str, item: &str, expected| {
        assert_boxes(
            container,
            &format!("writing-mode:horizontal-tb;flex:0 0 160px;max-width:80px;{item}"),
            "xxxx xxxx xxxx",
            expected,
        )
    };
    v(
        VERTICAL_COLUMN,
        "padding:5%;margin:3%",
        [[0., 0., 81.78125, 30.], [0.890625, 0.890625, 80., 28.21875]],
    );
    v(
        &format!("{VERTICAL_COLUMN};max-width:100px;padding:3px;border:2px solid"),
        "padding:5%;margin:3%",
        [[0., 0., 91.78125, 40.], [5.890625, 5.890625, 80., 28.21875]],
    );
    v(
        VERTICAL_COLUMN,
        "margin:3%",
        [[0., 0., 81.78125, 30.], [0.890625, 0.890625, 80., 28.21875]],
    );
    // Items in the writing mode of their container take the same margins.
    assert_boxes(
        VERTICAL_COLUMN,
        "flex:0 0 160px;max-width:80px;margin:3%",
        "xxxx xxxx xxxx",
        [[0., 0., 88.375, 140.], [4.1875, 4.1875, 80., 131.625]],
    );
    assert_boxes(
        HORIZONTAL_COLUMN,
        "flex:0 0 120px;max-height:80px;margin:3%",
        "xxxx xxxx xx",
        [[0., 0., 120., 87.1875], [3.59375, 3.59375, 112.8125, 80.]],
    );
}
