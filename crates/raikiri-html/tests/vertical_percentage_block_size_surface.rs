//! The `width` of a block-level child of a block container in a vertical
//! writing mode. For a vertical child it is the block size, and a percentage
//! of it refers to the block size of the container: while the container's
//! content decides that size, the percentage behaves as `auto` (CSS Sizing 3
//! §5.2.1), and so do percentage limits. For a horizontal child it is the
//! inline size of an orthogonal flow, which refers to the initial containing
//! block then (CSS Writing Modes 4 §7.3).
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

/// The border boxes, as `[x, y, width, height]`, of the elements with the
/// ids `o`, `m` and `i` in `body`, those that exist, in that order.
fn boxes(body: &str) -> Vec<[f32; 4]> {
    let html = format!(
        "<!DOCTYPE html><html><head><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:0;width:800px;font:10px/10px Ahem}}div{{box-sizing:border-box}}\
         </style></head><body>{body}</body></html>"
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
    ["o", "m", "i"]
        .iter()
        .filter_map(|id| {
            page.fragments()
                .find(|fragment| dom.attr(fragment.node(), "id") == Some(*id))
                .map(|fragment| {
                    let r = fragment.rect();
                    [r.x, r.y, r.width, r.height]
                })
        })
        .collect()
}

/// Asserts the boxes match Chromium's within 1/32px.
#[track_caller]
fn assert_boxes(body: &str, expected: &[[f32; 4]]) {
    let actual = boxes(body);
    let close = actual.len() == expected.len()
        && actual
            .iter()
            .flatten()
            .zip(expected.iter().flatten())
            .all(|(a, e)| (a - e).abs() <= 1.0 / 32.0);
    assert!(close, "{body}: {actual:?} != {expected:?}");
}

const NESTED: &str = "<div id=m><div id=i style=\"width:50%\">x</div></div>";
const DIRECT: &str = "<div id=i style=\"width:50%\">x</div>";

fn container(style: &str, content: &str) -> String {
    format!("<div id=o style=\"{style}\">{content}</div>")
}

#[test]
fn percentage_widths_in_a_content_sized_vertical_container_are_auto() {
    let square = [0., 0., 10., 10.];
    for style in [
        "display:inline-block;writing-mode:vertical-rl",
        "display:block;writing-mode:vertical-rl",
        "float:left;writing-mode:vertical-rl",
        "display:inline-block;writing-mode:vertical-lr",
        "position:absolute;writing-mode:vertical-rl",
    ] {
        assert_boxes(&container(style, NESTED), &[square; 3]);
        assert_boxes(&container(style, DIRECT), &[square; 2]);
    }
    assert_boxes(
        &container(
            "display:inline-block;writing-mode:vertical-rl",
            "<div id=i style=\"width:calc(50% + 4px)\">x</div>",
        ),
        &[square; 2],
    );
    // The limits of the container do not make its block size definite.
    assert_boxes(
        &container(
            "display:inline-block;min-width:100px;writing-mode:vertical-rl",
            DIRECT,
        ),
        &[[0., 0., 100., 10.], [90., 0., 10., 10.]],
    );
    // Nor does the size a sibling gives it.
    assert_boxes(
        "<div id=o style=\"display:inline-block;writing-mode:vertical-rl\">\
         <div id=m style=\"width:40px\">x</div><div id=i style=\"width:50%\">x</div></div>",
        &[[0., 0., 50., 10.], [10., 0., 40., 10.], [0., 0., 10., 10.]],
    );
    // A percentage limit of an indefinite block size limits nothing.
    assert_boxes(
        "<div id=o style=\"display:inline-block;writing-mode:vertical-rl\">\
         <div id=m>xxx</div><div id=i style=\"min-width:50%\">x</div></div>",
        &[[0., 0., 20., 30.], [10., 0., 10., 30.], [0., 0., 10., 30.]],
    );
}

#[test]
fn percentage_widths_refer_only_to_a_definite_block_size() {
    // The container's width is definite, but the auto width of the middle box
    // is not, whatever width the container gives it.
    for style in [
        "display:block;width:100px;writing-mode:vertical-rl",
        "display:inline-block;width:100px;writing-mode:vertical-rl",
        "display:inline-block;min-width:100px;writing-mode:vertical-rl",
        "display:inline-flex;flex-flow:column;min-width:100px;writing-mode:vertical-rl",
    ] {
        assert_boxes(
            &container(style, NESTED),
            &[
                [0., 0., 100., 10.],
                [90., 0., 10., 10.],
                [90., 0., 10., 10.],
            ],
        );
    }
    assert_boxes(
        &container("display:block;width:100px;writing-mode:vertical-rl", DIRECT),
        &[[0., 0., 100., 10.], [50., 0., 50., 10.]],
    );
    assert_boxes(
        &container(
            "display:inline-block;writing-mode:vertical-rl",
            "<div id=m style=\"width:60px\"><div id=i style=\"width:50%\">x</div></div>",
        ),
        &[[0., 0., 60., 10.], [0., 0., 60., 10.], [30., 0., 30., 10.]],
    );
    assert_boxes(
        &container(
            "display:block;width:100px;writing-mode:vertical-rl",
            "<div id=i style=\"min-width:60%\">x</div>",
        ),
        &[[0., 0., 100., 10.], [40., 0., 60., 10.]],
    );
    assert_boxes(
        &container(
            "display:block;width:100px;writing-mode:vertical-rl",
            "<div id=i style=\"max-width:5%;width:50px\">x</div>",
        ),
        &[[0., 0., 100., 10.], [95., 0., 5., 10.]],
    );
    // A flex item's flexed main size is definite.
    assert_boxes(
        "<div style=\"display:flex;width:300px\"><div id=o style=\"writing-mode:vertical-rl;\
         flex:1\"><div id=i style=\"width:50%\">x</div></div></div>",
        &[[0., 0., 300., 10.], [150., 0., 150., 10.]],
    );
}

#[test]
fn percentage_widths_of_orthogonal_children_refer_to_the_page_area() {
    assert_boxes(
        &container(
            "display:inline-block;writing-mode:vertical-rl",
            "<div id=i style=\"writing-mode:horizontal-tb;width:50%\">xx</div>",
        ),
        &[[0., 0., 400., 10.], [0., 0., 400., 10.]],
    );
    assert_boxes(
        &container(
            "display:block;width:100px;writing-mode:vertical-rl",
            "<div id=i style=\"writing-mode:horizontal-tb;width:50%\">xx</div>",
        ),
        &[[0., 0., 100., 10.], [50., 0., 50., 10.]],
    );
}

#[test]
fn percentage_offsets_of_relative_children_follow_the_same_bases() {
    let sibling = "<div id=m style=\"width:40px\">x</div>";
    let inline_block = "display:inline-block;writing-mode:vertical-rl";
    // A horizontal offset refers to the block size, `auto` while the content
    // decides it.
    for offset in ["left:50%", "right:50%"] {
        assert_boxes(
            &container(
                inline_block,
                &format!("{sibling}<div id=i style=\"position:relative;{offset}\">x</div>"),
            ),
            &[[0., 0., 50., 10.], [10., 0., 40., 10.], [0., 0., 10., 10.]],
        );
    }
    assert_boxes(
        &container(
            "display:block;width:100px;writing-mode:vertical-rl",
            "<div id=i style=\"position:relative;left:10%\">x</div>",
        ),
        &[[0., 0., 100., 10.], [100., 0., 10., 10.]],
    );
    // A vertical offset refers to the inline size, which the lines settle.
    assert_boxes(
        &container(
            inline_block,
            "<div id=m style=\"width:40px\">xxxx</div>\
             <div id=i style=\"position:relative;top:50%\">x</div>",
        ),
        &[[0., 0., 50., 40.], [10., 0., 40., 40.], [0., 20., 10., 40.]],
    );
}
