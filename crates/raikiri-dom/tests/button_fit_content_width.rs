//! The `auto` width of a `<button>`: its fit-content size, also where the button
//! is block-level, instead of the room its container offers.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px and `10px Ahem` text:
//! each is the element's left, top, width and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a case and the `(id, left, top, width, height)` it
/// expects.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f32, f32, f32, f32)]);

/// Lays out each case and compares the elements it names with the expected
/// boxes.
fn check_boxes(cases: &[Case<'_>]) {
    for &(label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, left, top, width, height) in expected {
            let (_, rect) = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            let want = [left, top, width, height];
            assert!(
                rect.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= TOLERANCE),
                "{label}: #{id} is {rect:?}, expected {want:?} (left, top, width, height)"
            );
        }
    }
}

/// A block-level button without its own padding, border or font.
const BUTTON: &str = "all: unset; display: block;";

/// A block-level button takes the size of its content, not the room of its
/// container, as an inline-block does.
#[test]
fn a_block_level_button_takes_the_width_of_its_content() {
    check_boxes(&[
        (
            "text",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON}'>xx</button></div>"
            ),
            &[("w", 8.0, 8.0, 200.0, 10.0), ("b", 8.0, 8.0, 20.0, 10.0)],
        ),
        (
            "no content",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} height: 20px'></button></div>"
            ),
            &[("b", 8.0, 8.0, 0.0, 20.0)],
        ),
        (
            "after a block",
            &format!(
                "<div id=w style='width: 200px'><div style='height: 5px'></div>\
                 <button id=b style='{BUTTON} height: 20px'>xx</button></div>"
            ),
            &[("w", 8.0, 8.0, 200.0, 25.0), ("b", 8.0, 13.0, 20.0, 20.0)],
        ),
        (
            "in a block of automatic width",
            &format!("<div id=w><button id=b style='{BUTTON} height: 20px'></button></div>"),
            &[("w", 8.0, 8.0, 784.0, 20.0), ("b", 8.0, 8.0, 0.0, 20.0)],
        ),
        (
            "two buttons",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} height: 20px'>xx</button>\
                 <button id=c style='{BUTTON} height: 20px'>xxxx</button></div>"
            ),
            &[("b", 8.0, 8.0, 20.0, 20.0), ("c", 8.0, 28.0, 40.0, 20.0)],
        ),
        (
            "text that wraps",
            &format!(
                "<div id=w style='width: 100px'><button id=b style='{BUTTON}'>xx xx xx xx xx xx</button></div>"
            ),
            &[("b", 8.0, 8.0, 100.0, 20.0)],
        ),
        (
            "padding",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} padding: 5px; height: 30px'>xx</button></div>"
            ),
            &[("b", 8.0, 8.0, 30.0, 40.0)],
        ),
        (
            "in a box that clips",
            &format!(
                "<div id=w style='width: 200px; overflow: hidden'><button id=b style='{BUTTON} height: 20px'>xx</button></div>"
            ),
            &[("b", 8.0, 8.0, 20.0, 20.0)],
        ),
        (
            "in a flow-root",
            &format!(
                "<div id=w style='width: 200px; display: flow-root'><button id=b style='{BUTTON} height: 20px'>xx</button></div>"
            ),
            &[("b", 8.0, 8.0, 20.0, 20.0)],
        ),
        (
            "in a table cell",
            &format!(
                "<table><tr><td id=w style='width: 200px; padding: 0'>\
                 <button id=b style='{BUTTON} height: 20px'>xx</button></td></tr></table>"
            ),
            &[
                ("w", 10.0, 10.0, 200.0, 20.0),
                ("b", 10.0, 10.0, 20.0, 20.0),
            ],
        ),
        (
            "beside a float",
            &format!(
                "<div id=w style='float: left'><button id=b style='{BUTTON}'>xxx</button></div>"
            ),
            &[("w", 8.0, 8.0, 30.0, 10.0), ("b", 8.0, 8.0, 30.0, 10.0)],
        ),
    ]);
}

/// The sizes the button states, and the margins around it, still apply to the
/// width it takes from its content.
#[test]
fn the_width_of_a_block_level_button_is_still_bounded_and_placed_by_its_styles() {
    check_boxes(&[
        (
            "margin auto on both sides",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} margin: 0 auto; height: 20px'>xx</button></div>"
            ),
            &[("b", 98.0, 8.0, 20.0, 20.0)],
        ),
        (
            "margin auto on the left",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} margin-left: auto; height: 20px'>xx</button></div>"
            ),
            &[("b", 188.0, 8.0, 20.0, 20.0)],
        ),
        (
            "right to left",
            &format!(
                "<div id=w style='width: 200px; direction: rtl'><button id=b style='{BUTTON} height: 20px'>xx</button></div>"
            ),
            &[("b", 188.0, 8.0, 20.0, 20.0)],
        ),
        (
            "max-width",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} max-width: 10px; height: 20px'>xx</button></div>"
            ),
            &[("b", 8.0, 8.0, 10.0, 20.0)],
        ),
        (
            "min-width",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} min-width: 100px; height: 20px'>xx</button></div>"
            ),
            &[("b", 8.0, 8.0, 100.0, 20.0)],
        ),
        (
            "width",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} width: 80px; height: 20px'></button></div>"
            ),
            &[("b", 8.0, 8.0, 80.0, 20.0)],
        ),
        (
            "width as a percentage",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} width: 100%; height: 20px'>xx</button></div>"
            ),
            &[("b", 8.0, 8.0, 200.0, 20.0)],
        ),
        (
            "width: stretch",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} width: stretch; height: 20px'>xx</button></div>"
            ),
            &[("b", 8.0, 8.0, 200.0, 20.0)],
        ),
        (
            "an aspect ratio and a minimum height",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} aspect-ratio: 1; min-height: 50px'></button></div>"
            ),
            &[("w", 8.0, 8.0, 200.0, 50.0), ("b", 8.0, 8.0, 50.0, 50.0)],
        ),
        (
            "an aspect ratio alone",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} aspect-ratio: 1'></button></div>"
            ),
            &[("w", 8.0, 8.0, 200.0, 0.0), ("b", 8.0, 8.0, 0.0, 0.0)],
        ),
    ]);
}

/// Mixed with text, a block-level button is a block child of the paragraph's
/// root, which sizes it the same way.
#[test]
fn a_block_level_button_among_text_takes_the_width_of_its_content() {
    check_boxes(&[
        (
            "after text",
            &format!(
                "<div id=w style='width: 200px'>aa<button id=b style='{BUTTON} height: 20px'>xx</button></div>"
            ),
            &[("w", 8.0, 8.0, 200.0, 30.0), ("b", 8.0, 18.0, 20.0, 20.0)],
        ),
        (
            "before text",
            &format!(
                "<div id=w style='width: 200px'><button id=b style='{BUTTON} height: 20px'>xx</button>aa</div>"
            ),
            &[("w", 8.0, 8.0, 200.0, 30.0), ("b", 8.0, 8.0, 20.0, 20.0)],
        ),
    ]);
}

/// A flex or grid item is sized by the container: a button that stretches
/// does so as any item does.
#[test]
fn a_button_that_is_a_flex_or_grid_item_still_stretches() {
    check_boxes(&[
        (
            "column flex",
            &format!(
                "<div id=w style='display: flex; flex-direction: column; width: 200px'><button id=b style='{BUTTON} height: 20px'></button></div>"
            ),
            &[("b", 8.0, 8.0, 200.0, 20.0)],
        ),
        (
            "grid",
            &format!(
                "<div id=w style='display: grid; width: 200px'><button id=b style='{BUTTON} height: 20px'></button></div>"
            ),
            &[("b", 8.0, 8.0, 200.0, 20.0)],
        ),
    ]);
}

/// A percentage height that is cyclic while the flex container is sized
/// resolves against the stretched item afterwards, so a button with an aspect
/// ratio ends up as tall as the item and overflows it.
#[test]
fn a_button_with_a_percentage_height_and_a_ratio_in_a_flex_item_overflows_the_stretched_item() {
    let item = |height: &str, extra: &str| {
        format!(
            "<div id=f style='display: flex'><div id=i style='background: green'>\
             <div id=a style='height: 50px; width: 100px'></div>\
             <button id=b style='{BUTTON} height: {height}; line-height: 0; aspect-ratio: 1; {extra}'></button>\
             </div></div>"
        )
    };
    check_boxes(&[
        (
            "100%",
            &item("100%", "min-height: 50px"),
            &[
                ("f", 8.0, 8.0, 784.0, 100.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
                ("b", 8.0, 58.0, 100.0, 100.0),
            ],
        ),
        (
            "90%",
            &item("90%", "min-height: 50px"),
            &[
                ("f", 8.0, 8.0, 784.0, 100.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
                ("b", 8.0, 58.0, 90.0, 90.0),
            ],
        ),
        (
            "auto",
            &item("auto", "min-height: 50px"),
            &[
                ("f", 8.0, 8.0, 784.0, 100.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
                ("b", 8.0, 58.0, 50.0, 50.0),
            ],
        ),
        (
            "100% without a minimum height",
            &item("100%", ""),
            &[
                ("f", 8.0, 8.0, 784.0, 50.0),
                ("i", 8.0, 8.0, 100.0, 50.0),
                ("b", 8.0, 58.0, 50.0, 50.0),
            ],
        ),
    ]);
}

/// A block-level button that is a direct child of a multicol container takes
/// the size of its content inside its column, as it does in a block.
#[test]
fn a_block_level_button_directly_in_a_multicol_takes_the_width_of_its_content() {
    let multicol = |style: &str, content: &str| {
        format!(
            "<div id=w style='columns: 2; width: 200px; column-gap: 0; {style}'>{content}</div>"
        )
    };
    check_boxes(&[
        (
            "text",
            &multicol("", &format!("<button id=b style='{BUTTON}'>xx</button>")),
            &[("w", 8.0, 8.0, 200.0, 10.0), ("b", 8.0, 8.0, 20.0, 10.0)],
        ),
        (
            "no content",
            &multicol(
                "",
                &format!("<button id=b style='{BUTTON} height: 20px'></button>"),
            ),
            &[("w", 8.0, 8.0, 200.0, 20.0), ("b", 8.0, 8.0, 0.0, 20.0)],
        ),
        (
            "after a paragraph",
            &multicol(
                "",
                &format!(
                    "<p id=p style='margin: 0'>aa</p><button id=b style='{BUTTON} height: 20px'>xx</button>"
                ),
            ),
            &[("p", 8.0, 8.0, 100.0, 10.0), ("b", 108.0, 8.0, 20.0, 20.0)],
        ),
        (
            "text that wraps in the column",
            &multicol(
                "height: 50px; column-fill: auto",
                &format!("<button id=b style='{BUTTON}'>xx xx xx xx xx xx xx</button>"),
            ),
            &[("w", 8.0, 8.0, 200.0, 50.0), ("b", 8.0, 8.0, 100.0, 30.0)],
        ),
        (
            "text that cannot wrap",
            &multicol(
                "",
                &format!(
                    "<button id=b style='{BUTTON} white-space: nowrap'>xxxx xxxx xxxx</button>"
                ),
            ),
            &[("b", 8.0, 8.0, 140.0, 10.0)],
        ),
        (
            "padding",
            &multicol(
                "",
                &format!("<button id=b style='{BUTTON} padding: 5px; height: 30px'>xx</button>"),
            ),
            &[("b", 8.0, 8.0, 30.0, 40.0)],
        ),
        (
            "margin auto on both sides",
            &multicol(
                "",
                &format!("<button id=b style='{BUTTON} margin: 0 auto; height: 20px'>xx</button>"),
            ),
            &[("b", 48.0, 8.0, 20.0, 20.0)],
        ),
        (
            "right to left",
            &multicol(
                "direction: rtl",
                &format!("<button id=b style='{BUTTON} height: 20px'>xx</button>"),
            ),
            &[("b", 188.0, 8.0, 20.0, 20.0)],
        ),
        (
            "a column width instead of a count",
            &format!(
                "<div id=w style='column-width: 90px; width: 200px; column-gap: 0'>\
                 <button id=b style='{BUTTON} height: 20px'>xx</button></div>"
            ),
            &[("b", 8.0, 8.0, 20.0, 20.0)],
        ),
        (
            "two buttons in two columns",
            &multicol(
                "",
                &format!(
                    "<button id=b style='{BUTTON} height: 20px'>xx</button>\
                     <button id=c style='{BUTTON} height: 20px'>xxxx</button>"
                ),
            ),
            &[("b", 8.0, 8.0, 20.0, 20.0), ("c", 108.0, 8.0, 40.0, 20.0)],
        ),
        (
            "a fixed height",
            &multicol(
                "height: 50px; column-fill: auto",
                &format!(
                    "<button id=b style='{BUTTON} height: 20px'>xx</button>\
                     <button id=c style='{BUTTON} height: 20px'>xxxx</button>"
                ),
            ),
            &[("b", 8.0, 8.0, 20.0, 20.0), ("c", 8.0, 28.0, 40.0, 20.0)],
        ),
        (
            "a spanner",
            &multicol(
                "",
                &format!(
                    "<button id=b style='{BUTTON} height: 20px; column-span: all'>xx</button>"
                ),
            ),
            &[("b", 8.0, 8.0, 20.0, 20.0)],
        ),
    ]);
    // The margins narrow the room the text wraps in, the column width minus
    // the margins.
    let wrapping = "height: 50px; column-fill: auto";
    let words = "xx xx xx xx xx xx xx";
    check_boxes(&[
        (
            "margins on both sides",
            &multicol(
                wrapping,
                &format!("<button id=b style='{BUTTON} margin: 0 10px'>{words}</button>"),
            ),
            &[("b", 18.0, 8.0, 80.0, 30.0)],
        ),
        (
            "a margin on the left only",
            &multicol(
                wrapping,
                &format!("<button id=b style='{BUTTON} margin-left: 10px'>{words}</button>"),
            ),
            &[("b", 18.0, 8.0, 90.0, 30.0)],
        ),
        (
            "margins as a percentage of the column",
            &multicol(
                wrapping,
                &format!("<button id=b style='{BUTTON} margin: 0 10%'>{words}</button>"),
            ),
            &[("b", 18.0, 8.0, 80.0, 30.0)],
        ),
        (
            "margins and text that fits",
            &multicol(
                "",
                &format!("<button id=b style='{BUTTON} margin: 0 10px; height: 20px'>xx</button>"),
            ),
            &[("b", 18.0, 8.0, 20.0, 20.0)],
        ),
        (
            "margins after a paragraph",
            &multicol(
                wrapping,
                &format!(
                    "<p style='margin: 0'>xx</p>\
                     <button id=b style='{BUTTON} margin: 0 10px'>{words}</button>"
                ),
            ),
            &[("b", 18.0, 18.0, 80.0, 30.0)],
        ),
        (
            "margins on a spanner",
            &multicol(
                "",
                &format!(
                    "<button id=b style='{BUTTON} margin: 0 10px; column-span: all'>{words}</button>"
                ),
            ),
            &[("b", 18.0, 8.0, 180.0, 20.0)],
        ),
    ]);
    // The sizes the button states still apply.
    check_boxes(&[
        (
            "width",
            &multicol(
                "",
                &format!("<button id=b style='{BUTTON} width: 80px; height: 20px'></button>"),
            ),
            &[("b", 8.0, 8.0, 80.0, 20.0)],
        ),
        (
            "width as a percentage of the column",
            &multicol(
                "",
                &format!("<button id=b style='{BUTTON} width: 100%; height: 20px'>xx</button>"),
            ),
            &[("b", 8.0, 8.0, 100.0, 20.0)],
        ),
        (
            "max-width",
            &multicol(
                "",
                &format!("<button id=b style='{BUTTON} max-width: 10px; height: 20px'>xx</button>"),
            ),
            &[("b", 8.0, 8.0, 10.0, 20.0)],
        ),
        (
            "min-width",
            &multicol(
                "",
                &format!("<button id=b style='{BUTTON} min-width: 60px; height: 20px'>xx</button>"),
            ),
            &[("b", 8.0, 8.0, 60.0, 20.0)],
        ),
    ]);
}
