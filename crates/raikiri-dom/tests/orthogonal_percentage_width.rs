//! A horizontal box in a vertical container whose width is open is an
//! orthogonal flow: a percentage of its width refers to the width of the
//! initial containing block, within a length `max-width` of the container
//! (CSS Writing Modes 4 §7.3).
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with no body margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
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
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

/// The parent's width is open, so a percentage of it falls back to the
/// initial containing block, 800px here.
const PARENT: &str = "writing-mode:vertical-rl;height:300px";

fn in_parent(parent: &str, child: &str) -> String {
    format!("<div id=p style=\"{parent}\">{child}</div>")
}

#[test]
fn a_percentage_width_falls_back_to_the_initial_containing_block() {
    check(
        &in_parent(
            PARENT,
            "<div id=c style=\"writing-mode:horizontal-tb;width:50%;height:20px\"></div>",
        ),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 400.0, 20.0)),
        ],
    );
    check(
        &in_parent(
            PARENT,
            "<div id=a style=\"writing-mode:horizontal-tb;width:calc(50% + 10px);height:20px\"></div>\
             <div id=b style=\"writing-mode:horizontal-tb;width:10px;min-width:25%;height:20px\"></div>",
        ),
        &[
            ("p", (0.0, 0.0, 610.0, 300.0)),
            ("a", (200.0, 0.0, 410.0, 20.0)),
            ("b", (0.0, 0.0, 200.0, 20.0)),
        ],
    );
}

#[test]
fn a_float_and_a_table_take_the_same_fallback() {
    for display in ["float:left", "display:table"] {
        check(
            &in_parent(
                PARENT,
                &format!(
                    "<div id=c style=\"{display};writing-mode:horizontal-tb;width:50%;height:20px\"></div>"
                ),
            ),
            &[
                ("p", (0.0, 0.0, 400.0, 300.0)),
                ("c", (0.0, 0.0, 400.0, 20.0)),
            ],
        );
    }
}

#[test]
fn a_length_max_width_of_the_parent_caps_the_fallback() {
    check(
        &in_parent(
            "writing-mode:vertical-rl;height:300px;max-width:100px",
            "<div id=c style=\"writing-mode:horizontal-tb;width:50%;height:20px\"></div>",
        ),
        &[
            ("p", (0.0, 0.0, 50.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 20.0)),
        ],
    );
    // A larger `min-width` raises the cap.
    check(
        &in_parent(
            "writing-mode:vertical-rl;height:300px;max-width:100px;min-width:300px",
            "<div id=c style=\"writing-mode:horizontal-tb;width:50%;height:20px\"></div>",
        ),
        &[
            ("p", (0.0, 0.0, 300.0, 300.0)),
            ("c", (150.0, 0.0, 150.0, 20.0)),
        ],
    );
    // The cap is the content box of a border-box limit.
    check(
        &in_parent(
            "writing-mode:vertical-rl;height:300px;max-width:100px;padding:0 5px",
            "<div id=c style=\"writing-mode:horizontal-tb;width:50%;height:20px\"></div>",
        ),
        &[
            ("p", (0.0, 0.0, 55.0, 300.0)),
            ("c", (5.0, 0.0, 45.0, 20.0)),
        ],
    );
    // A percentage limit does not cap it.
    check(
        &in_parent(
            "writing-mode:vertical-rl;height:300px;max-width:50%",
            "<div id=c style=\"writing-mode:horizontal-tb;width:50%;height:20px\"></div>",
        ),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 400.0, 20.0)),
        ],
    );
}

#[test]
fn a_child_of_a_vertical_paragraph_takes_the_capped_fallback() {
    let inline_block = "X<div id=c style=\"display:inline-block;writing-mode:horizontal-tb;width:50%;height:20px\"></div>";
    check(
        &in_parent(PARENT, inline_block),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 10.0, 400.0, 20.0)),
        ],
    );
    check(
        &in_parent(
            "writing-mode:vertical-rl;height:300px;max-width:100px",
            inline_block,
        ),
        &[
            ("p", (0.0, 0.0, 50.0, 300.0)),
            ("c", (0.0, 10.0, 50.0, 20.0)),
        ],
    );
    check(
        &in_parent(
            "writing-mode:vertical-rl;height:300px;max-width:100px",
            "X<div id=c style=\"writing-mode:horizontal-tb;width:50%;height:20px\"></div>",
        ),
        &[
            ("p", (0.0, 0.0, 60.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 20.0)),
        ],
    );
}

#[test]
fn a_relative_offset_resolves_against_the_resolved_width() {
    check(
        &in_parent(
            PARENT,
            "<div id=c style=\"writing-mode:horizontal-tb;width:50%\">\
             <div id=f style=\"position:relative;left:50%;width:10px;height:10px\"></div></div>",
        ),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 400.0, 10.0)),
            ("f", (200.0, 0.0, 10.0, 10.0)),
        ],
    );
}
