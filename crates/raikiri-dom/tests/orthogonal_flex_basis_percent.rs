//! A percentage `flex-basis` of an item that runs in the other writing
//! direction than its column flex container refers to the initial containing
//! block along the main axis while the container's own main size is open: the
//! page area's height, or its width for a container in a vertical writing mode
//! (CSS Writing Modes 4 §7.3.1). A horizontal item of a horizontal container
//! still takes its content size there, and a container with a size of its own
//! still gives the percentage that size (CSS Flexbox 1 §9.2.3). The expected
//! boxes are the ones Chromium lays out for the same markup on an 800x600 page.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::HashMap;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border box `(x, y, width, height)` of every element with an `id` on the
/// first page of an 800x600 document that holds `body`.
fn boxes(body: &str) -> HashMap<String, [f32; 4]> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
    let mut found = HashMap::new();
    for fragment in page.fragments() {
        let Some(id) = dom.attr(fragment.node(), "id") else {
            continue;
        };
        let rect = fragment.rect();
        found
            .entry(id.to_owned())
            .or_insert([rect.x, rect.y, rect.width, rect.height]);
    }
    found
}

#[track_caller]
fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    let found = boxes(body);
    for (id, want) in expected {
        let got = found.get(*id).unwrap_or_else(|| panic!("no box `{id}`"));
        assert!(
            got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.01),
            "`{id}` is {got:?}, expected {want:?} for {body}"
        );
    }
}

/// An item in a vertical writing mode, `style` added, in a column flex
/// container whose own style is `container`.
fn vertical_item(container: &str, style: &str) -> String {
    format!(
        "<div id=c style=\"display:flex;flex-direction:column;{container}\">\
         <div id=a style=\"writing-mode:vertical-rl;{style}\">xxxx xxxx xx</div></div>"
    )
}

#[test]
fn flex_shorthand_percentages_refer_to_the_page_height() {
    for (basis, height) in [
        ("10%", 60.0),
        ("25%", 150.0),
        ("50%", 300.0),
        ("100%", 600.0),
    ] {
        for extra in ["", ";height:min-content"] {
            assert_boxes(
                &vertical_item("", &format!("flex:0 0 {basis}{extra}")),
                &[
                    ("c", [0.0, 0.0, 800.0, height]),
                    ("a", [0.0, 0.0, 800.0, height]),
                ],
            );
        }
    }
}

#[test]
fn flex_basis_alone_refers_to_the_page_height() {
    for (basis, height) in [("25%", 150.0), ("50%", 300.0), ("100%", 600.0)] {
        assert_boxes(
            &vertical_item("", &format!("flex-basis:{basis}")),
            &[
                ("c", [0.0, 0.0, 800.0, height]),
                ("a", [0.0, 0.0, 800.0, height]),
            ],
        );
    }
}

#[test]
fn calc_with_a_percentage_refers_to_the_page_height() {
    assert_boxes(
        &vertical_item("", "flex:0 0 calc(10% + 5px)"),
        &[
            ("c", [0.0, 0.0, 800.0, 65.0]),
            ("a", [0.0, 0.0, 800.0, 65.0]),
        ],
    );
}

#[test]
fn the_box_model_decides_what_the_basis_measures() {
    // `border-box` here: the padding and border are inside the 60px.
    assert_boxes(
        &vertical_item("", "flex:0 0 10%;padding:5px;border:3px solid"),
        &[("a", [0.0, 0.0, 800.0, 60.0])],
    );
    // `content-box`: the padding comes on top.
    assert_boxes(
        &vertical_item("", "flex:0 0 10%;box-sizing:content-box;padding:5px"),
        &[("a", [0.0, 0.0, 800.0, 70.0])],
    );
}

#[test]
fn the_min_height_of_the_item_still_applies() {
    assert_boxes(
        &vertical_item("", "flex:0 0 10%;min-height:100px"),
        &[
            ("c", [0.0, 0.0, 800.0, 100.0]),
            ("a", [0.0, 0.0, 800.0, 100.0]),
        ],
    );
}

#[test]
fn a_vertical_lr_item_and_a_reversed_container_resolve_alike() {
    assert_boxes(
        &vertical_item("", "writing-mode:vertical-lr;flex:0 0 10%"),
        &[("a", [0.0, 0.0, 800.0, 60.0])],
    );
    assert_boxes(
        &vertical_item("flex-direction:column-reverse", "flex:0 0 10%"),
        &[
            ("c", [0.0, 0.0, 800.0, 60.0]),
            ("a", [0.0, 0.0, 800.0, 60.0]),
        ],
    );
}

#[test]
fn each_item_resolves_its_own_percentage() {
    assert_boxes(
        "<div id=c style=\"display:flex;flex-direction:column\">\
         <div id=a style=\"writing-mode:vertical-rl;flex:0 0 10%\">xxxx xxxx xx</div>\
         <div id=b style=\"writing-mode:vertical-rl;flex:0 0 20%\">xx</div></div>",
        &[
            ("c", [0.0, 0.0, 800.0, 180.0]),
            ("a", [0.0, 0.0, 800.0, 60.0]),
            ("b", [0.0, 60.0, 800.0, 120.0]),
        ],
    );
    // A horizontal sibling keeps its own size.
    assert_boxes(
        "<div id=c style=\"display:flex;flex-direction:column\">\
         <div id=a style=\"writing-mode:vertical-rl;flex:0 0 10%\">xxxx xxxx xx</div>\
         <div id=b style=\"height:30px\">yy</div></div>",
        &[
            ("c", [0.0, 0.0, 800.0, 90.0]),
            ("a", [0.0, 0.0, 800.0, 60.0]),
            ("b", [0.0, 60.0, 800.0, 30.0]),
        ],
    );
}

#[test]
fn a_container_in_a_narrower_block_or_with_a_width_resolves_the_same() {
    assert_boxes(
        &format!(
            "<div id=w style=\"width:400px\">{}</div>",
            vertical_item("", "flex:0 0 10%")
        ),
        &[
            ("w", [0.0, 0.0, 400.0, 60.0]),
            ("c", [0.0, 0.0, 400.0, 60.0]),
            ("a", [0.0, 0.0, 400.0, 60.0]),
        ],
    );
    assert_boxes(
        &vertical_item("width:300px", "flex:0 0 10%"),
        &[
            ("c", [0.0, 0.0, 300.0, 60.0]),
            ("a", [0.0, 0.0, 300.0, 60.0]),
        ],
    );
}

#[test]
fn a_container_with_a_height_of_its_own_is_the_basis() {
    // 10% of 200px is 20px, below the 40px of the longest word.
    assert_boxes(
        &vertical_item("height:200px", "flex:0 0 10%"),
        &[
            ("c", [0.0, 0.0, 800.0, 200.0]),
            ("a", [0.0, 0.0, 800.0, 40.0]),
        ],
    );
    // A `min-height` leaves the height open, so the page decides the basis.
    assert_boxes(
        &vertical_item("min-height:200px", "flex:0 0 10%"),
        &[
            ("c", [0.0, 0.0, 800.0, 200.0]),
            ("a", [0.0, 0.0, 800.0, 60.0]),
        ],
    );
}

#[test]
fn a_horizontal_item_and_a_row_container_are_left_alone() {
    // A horizontal item in a column container with an open height: content.
    assert_boxes(
        "<div id=c style=\"display:flex;flex-direction:column\">\
         <div id=a style=\"flex:0 0 10%\">xxxx</div></div>",
        &[
            ("c", [0.0, 0.0, 800.0, 10.0]),
            ("a", [0.0, 0.0, 800.0, 10.0]),
        ],
    );
    // A row container: the main axis is the width of the container.
    assert_boxes(
        "<div id=c style=\"display:flex\">\
         <div id=a style=\"writing-mode:vertical-rl;flex:0 0 10%\">xxxx xxxx xx</div></div>",
        &[
            ("c", [0.0, 0.0, 800.0, 120.0]),
            ("a", [0.0, 0.0, 80.0, 120.0]),
        ],
    );
}

#[test]
fn a_container_in_a_vertical_writing_mode_refers_to_the_page_width() {
    // The main axis of the column runs across the page, so 10% is 80px of the
    // 800px page, whichever way the block axis runs.
    for mode in ["vertical-rl", "vertical-lr"] {
        assert_boxes(
            &format!(
                "<div id=c style=\"display:flex;flex-direction:column;writing-mode:{mode}\">\
                 <div id=a style=\"writing-mode:horizontal-tb;flex:0 0 10%\">xxxx</div></div>"
            ),
            &[("c", [0.0, 0.0, 80.0, 10.0]), ("a", [0.0, 0.0, 80.0, 10.0])],
        );
    }
    // With a width of its own the container is the basis again.
    assert_boxes(
        "<div id=c style=\"display:flex;flex-direction:column;writing-mode:vertical-rl;width:400px\">\
         <div id=a style=\"writing-mode:horizontal-tb;flex:0 0 10%\">xxxx</div></div>",
        &[
            ("c", [0.0, 0.0, 400.0, 10.0]),
            ("a", [360.0, 0.0, 40.0, 10.0]),
        ],
    );
}
