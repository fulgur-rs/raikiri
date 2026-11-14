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

#[test]
fn an_auto_width_fits_within_the_max_width_of_the_parent() {
    check(
        &in_parent(
            "writing-mode:vertical-rl;height:300px;max-width:100px",
            "<div id=c style=\"writing-mode:horizontal-tb\">XXXX XXXX XXXX XXXX XXXX</div>",
        ),
        &[
            ("p", (0.0, 0.0, 100.0, 300.0)),
            ("c", (0.0, 0.0, 100.0, 30.0)),
        ],
    );
}

#[test]
fn a_definite_width_of_the_parent_is_the_basis() {
    // A column flex container stretches the parent to a definite width.
    check(
        "<div style=\"display:flex;flex-direction:column;width:300px;height:300px\">\
         <div id=p style=\"writing-mode:vertical-rl\">\
         <div id=c style=\"writing-mode:horizontal-tb;width:50%;height:20px\"></div></div></div>",
        &[
            ("p", (0.0, 0.0, 300.0, 20.0)),
            ("c", (150.0, 0.0, 150.0, 20.0)),
        ],
    );
}

#[test]
fn a_percentage_margin_resolves_against_the_parent_height() {
    check(
        &in_parent(
            PARENT,
            "<div id=c style=\"writing-mode:horizontal-tb;margin-top:10%;width:20px;height:20px\"></div>",
        ),
        &[
            ("p", (0.0, 0.0, 20.0, 300.0)),
            ("c", (0.0, 30.0, 20.0, 20.0)),
        ],
    );
}

/// A child with a percentage width inside the vertical parent `p`, whose
/// style is `parent`, inside `outer`.
fn nested(outer: &str, parent: &str) -> String {
    format!(
        "<div style=\"{outer}\"><div id=p style=\"writing-mode:vertical-rl;{parent}\">\
         <div id=c style=\"writing-mode:horizontal-tb;width:50%;height:20px\"></div></div></div>"
    )
}

#[test]
fn a_flex_item_that_keeps_its_content_width_leaves_it_open() {
    let open = [
        ("p", (0.0, 0.0, 400.0, 300.0)),
        ("c", (0.0, 0.0, 400.0, 20.0)),
    ];
    // The content size, and a basis or a flexed size the automatic minimum
    // raises to it.
    check(&nested("display:flex;height:300px", ""), &open);
    check(
        &nested("display:flex;width:500px;height:300px", "flex:0 0 300px"),
        &open,
    );
    check(
        &nested("display:flex;width:400px;height:300px", "flex:1"),
        &open,
    );
    // The same within the parent's `max-width`.
    check(
        &nested("display:flex;height:300px", "max-width:100px"),
        &[
            ("p", (0.0, 0.0, 50.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 20.0)),
        ],
    );
    // A column container that does not stretch the item.
    check(
        &nested(
            "display:flex;flex-direction:column;width:500px;height:300px;align-items:start",
            "",
        ),
        &[
            ("p", (0.0, 0.0, 400.0, 20.0)),
            ("c", (0.0, 0.0, 400.0, 20.0)),
        ],
    );
}

#[test]
fn a_flexed_flex_item_width_is_the_basis() {
    check(
        &nested("display:flex;width:500px;height:300px", "flex:1"),
        &[
            ("p", (0.0, 0.0, 500.0, 300.0)),
            ("c", (250.0, 0.0, 250.0, 20.0)),
        ],
    );
    check(
        &nested("display:flex;width:500px;height:300px", "flex:0 0 450px"),
        &[
            ("p", (0.0, 0.0, 450.0, 300.0)),
            ("c", (225.0, 0.0, 225.0, 20.0)),
        ],
    );
    check(
        &nested("display:flex;width:300px;height:300px", "min-width:0"),
        &[
            ("p", (0.0, 0.0, 300.0, 300.0)),
            ("c", (150.0, 0.0, 150.0, 20.0)),
        ],
    );
}

#[test]
fn a_grid_item_follows_its_area_only_when_stretched() {
    check(
        &nested("display:grid;grid-template-columns:300px;height:300px", ""),
        &[
            ("p", (0.0, 0.0, 300.0, 300.0)),
            ("c", (150.0, 0.0, 150.0, 20.0)),
        ],
    );
    check(
        &nested(
            "display:grid;grid-template-columns:300px;height:300px;justify-items:start",
            "",
        ),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 400.0, 20.0)),
        ],
    );
    // An auto margin keeps it from stretching too.
    let area = "display:grid;grid-template-columns:500px;height:300px";
    check(
        &nested(area, "margin-left:auto"),
        &[
            ("p", (100.0, 0.0, 400.0, 300.0)),
            ("c", (100.0, 0.0, 400.0, 20.0)),
        ],
    );
    check(
        &nested(area, "margin-right:auto"),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 400.0, 20.0)),
        ],
    );
}

#[test]
fn an_absolutely_positioned_parent_that_shrinks_to_fit_leaves_its_width_open() {
    let outer = "position:relative;width:500px;height:300px";
    check(
        &nested(outer, "position:absolute;height:300px"),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 400.0, 20.0)),
        ],
    );
    check(
        &nested(outer, "position:absolute;height:300px;max-width:100px"),
        &[
            ("p", (0.0, 0.0, 50.0, 300.0)),
            ("c", (0.0, 0.0, 50.0, 20.0)),
        ],
    );
    // Insets on both sides give it a definite width.
    check(
        &nested(outer, "position:absolute;left:0;right:0;height:300px"),
        &[
            ("p", (0.0, 0.0, 500.0, 300.0)),
            ("c", (250.0, 0.0, 250.0, 20.0)),
        ],
    );
}

#[test]
fn a_vertical_paragraph_flex_item_follows_the_same_rule() {
    let inline_block = "X<div id=c style=\"display:inline-block;writing-mode:horizontal-tb;width:50%;height:20px\"></div>";
    let block = "X<div id=c style=\"writing-mode:horizontal-tb;width:50%;height:20px\"></div>";
    let item = |outer: &str, parent: &str, child: &str| {
        format!(
            "<div style=\"{outer}\"><div id=p style=\"writing-mode:vertical-rl;{parent}\">{child}</div></div>"
        )
    };
    // The content size stays open.
    check(
        &item("display:flex;height:300px", "", inline_block),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 10.0, 400.0, 20.0)),
        ],
    );
    check(
        &item("display:flex;height:300px", "", block),
        &[
            ("p", (0.0, 0.0, 410.0, 300.0)),
            ("c", (0.0, 0.0, 400.0, 20.0)),
        ],
    );
    // A flexed size is the basis.
    check(
        &item(
            "display:flex;width:500px;height:300px",
            "flex:1",
            inline_block,
        ),
        &[
            ("p", (0.0, 0.0, 500.0, 300.0)),
            ("c", (250.0, 10.0, 250.0, 20.0)),
        ],
    );
    check(
        &item("display:flex;width:500px;height:300px", "flex:1", block),
        &[
            ("p", (0.0, 0.0, 500.0, 300.0)),
            ("c", (240.0, 0.0, 250.0, 20.0)),
        ],
    );
}

#[test]
fn a_width_that_only_matches_the_content_size_is_classified_by_how_it_was_given() {
    // A stretched grid item is definite even when the area is as wide as
    // the content.
    check(
        &nested("display:grid;grid-template-columns:400px;height:300px", ""),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (200.0, 0.0, 200.0, 20.0)),
        ],
    );
    // A flex basis that the automatic minimum meets stays open.
    check(
        &nested("display:flex;width:600px;height:300px", "flex:0 0 400px"),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (0.0, 0.0, 400.0, 20.0)),
        ],
    );
    // Without an automatic minimum, a flexed size is definite even when it
    // comes to the content size.
    check(
        &nested(
            "display:flex;width:400px;height:300px",
            "flex:1 1 0;min-width:0",
        ),
        &[
            ("p", (0.0, 0.0, 400.0, 300.0)),
            ("c", (200.0, 0.0, 200.0, 20.0)),
        ],
    );
    // An item that a column container centres keeps its content width.
    check(
        &nested(
            "display:flex;flex-direction:column;width:500px;height:300px;align-items:center",
            "",
        ),
        &[
            ("p", (50.0, 0.0, 400.0, 20.0)),
            ("c", (50.0, 0.0, 400.0, 20.0)),
        ],
    );
    // A ratio takes a definite width from a stated height.
    check(
        &nested(
            "display:flex;height:300px;align-items:start",
            "height:300px;aspect-ratio:1",
        ),
        &[
            ("p", (0.0, 0.0, 300.0, 300.0)),
            ("c", (150.0, 0.0, 150.0, 20.0)),
        ],
    );
}
