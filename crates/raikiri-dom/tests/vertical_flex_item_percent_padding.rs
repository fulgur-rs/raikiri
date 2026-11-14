//! A vertical flex container that is an item of a flex container resolves its
//! percentage padding against the inline size of its containing block (CSS
//! Writing Modes 4 §7.1, CSS Box 4 §4): the width of a horizontal flex
//! container, the height of a vertical one. Its parent measures it for a flex
//! base size and an automatic minimum size without a size along the main
//! axis, and the item, as a flex container of its own items, records its own
//! size meanwhile; the padding still refers to the parent's size. The expected
//! boxes are the ones Chromium lays out for the same markup on an 800x600
//! page.

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

/// Three horizontal items with flex bases of 40px, 50px and 30px; the first
/// two grow.
const ITEMS: &str = "<div id=a style=\"writing-mode:horizontal-tb;flex:1 0 40px\">a</div>\
     <div id=b style=\"writing-mode:horizontal-tb;flex:1 0 50px\">b</div>\
     <div id=d style=\"writing-mode:horizontal-tb;flex:0 0 30px\">d</div>";

/// The border box `(x, y, width, height)` of every element with an `id` on the
/// first page of an 800x600 document that holds `body`.
fn boxes(body: &str) -> HashMap<String, [f32; 4]> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
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
        if let Some(id) = dom.attr(fragment.node(), "id") {
            let rect = fragment.rect();
            found.insert(id.to_string(), [rect.x, rect.y, rect.width, rect.height]);
        }
    }
    found
}

/// One case: the style of the parent, the style of the container `c`, and the
/// expected border box of each element by its `id`.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, [f32; 4])]);

/// Lay out each `(parent, container, expected)` case, a container `c` holding
/// [`ITEMS`] in a parent with the given style, and compare the boxes.
fn assert_cases(cases: &[Case<'_>]) {
    let mut failures = Vec::new();
    for (parent, container, expected) in cases {
        let found = boxes(&format!(
            "<div style=\"{parent}\"><div id=c style=\"{container}\">{ITEMS}</div></div>"
        ));
        for (id, want) in *expected {
            let got = found.get(*id).copied();
            let matches = got.is_some_and(|got| {
                got.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= 0.02)
            });
            if !matches {
                failures.push(format!(
                    "{parent} / {container}: {id} expected {want:?}, got {got:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_vertical_flex_item_resolves_its_percentage_padding_against_its_flex_container() {
    // With `padding: 0 10%` in a 300px wide horizontal parent, the item has
    // 30px of padding on each side; with `padding: 5%` in a 200px tall
    // vertical parent, 10px on every side.
    assert_cases(&[
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:flex;padding:0 10%",
            &[
                ("c", [0.0, 0.0, 70.0, 120.0]),
                ("a", [30.0, 0.0, 10.0, 40.0]),
                ("b", [30.0, 40.0, 10.0, 50.0]),
                ("d", [30.0, 90.0, 10.0, 30.0]),
            ],
        ),
        (
            "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:flex;padding:5%",
            &[
                ("c", [270.0, 0.0, 30.0, 140.0]),
                ("a", [280.0, 10.0, 10.0, 40.0]),
                ("b", [280.0, 50.0, 10.0, 50.0]),
                ("d", [280.0, 100.0, 10.0, 30.0]),
            ],
        ),
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;padding:0 10%",
            &[
                ("c", [0.0, 0.0, 70.0, 120.0]),
                ("a", [30.0, 0.0, 10.0, 40.0]),
                ("b", [30.0, 40.0, 10.0, 50.0]),
                ("d", [30.0, 90.0, 10.0, 30.0]),
            ],
        ),
        (
            "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;padding:5%",
            &[
                ("c", [270.0, 0.0, 30.0, 140.0]),
                ("a", [280.0, 10.0, 10.0, 40.0]),
                ("b", [280.0, 50.0, 10.0, 50.0]),
                ("d", [280.0, 100.0, 10.0, 30.0]),
            ],
        ),
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-direction:column;padding:0 10%",
            &[
                ("c", [0.0, 0.0, 180.0, 10.0]),
                ("a", [110.0, 0.0, 40.0, 10.0]),
                ("b", [60.0, 0.0, 50.0, 10.0]),
                ("d", [30.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-direction:column;padding:5%",
            &[
                ("c", [160.0, 0.0, 140.0, 30.0]),
                ("a", [250.0, 10.0, 40.0, 10.0]),
                ("b", [200.0, 10.0, 50.0, 10.0]),
                ("d", [170.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-direction:column;flex-wrap:wrap;max-width:100px;padding:0 10%",
            &[
                ("c", [0.0, 0.0, 150.0, 20.0]),
                ("a", [80.0, 0.0, 40.0, 10.0]),
                ("b", [30.0, 0.0, 50.0, 10.0]),
                ("d", [90.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-direction:column;flex-wrap:wrap;max-width:100px;padding:5%",
            &[
                ("c", [190.0, 0.0, 110.0, 40.0]),
                ("a", [250.0, 10.0, 40.0, 10.0]),
                ("b", [200.0, 10.0, 50.0, 10.0]),
                ("d", [260.0, 20.0, 30.0, 10.0]),
            ],
        ),
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-direction:column;flex-wrap:wrap;max-height:100px;padding:0 10%",
            &[
                ("c", [0.0, 0.0, 180.0, 10.0]),
                ("a", [110.0, 0.0, 40.0, 10.0]),
                ("b", [60.0, 0.0, 50.0, 10.0]),
                ("d", [30.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-direction:column;flex-wrap:wrap;max-height:100px;padding:5%",
            &[
                ("c", [160.0, 0.0, 140.0, 30.0]),
                ("a", [250.0, 10.0, 40.0, 10.0]),
                ("b", [200.0, 10.0, 50.0, 10.0]),
                ("d", [170.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:flex;max-width:100px;padding:0 10%",
            &[
                ("c", [0.0, 0.0, 70.0, 120.0]),
                ("a", [30.0, 0.0, 10.0, 40.0]),
                ("b", [30.0, 40.0, 10.0, 50.0]),
                ("d", [30.0, 90.0, 10.0, 30.0]),
            ],
        ),
        (
            "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:flex;max-width:100px;padding:5%",
            &[
                ("c", [270.0, 0.0, 30.0, 140.0]),
                ("a", [280.0, 10.0, 10.0, 40.0]),
                ("b", [280.0, 50.0, 10.0, 50.0]),
                ("d", [280.0, 100.0, 10.0, 30.0]),
            ],
        ),
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-wrap:wrap;max-width:100px;padding:0 10%",
            &[
                ("c", [0.0, 0.0, 70.0, 120.0]),
                ("a", [30.0, 0.0, 10.0, 40.0]),
                ("b", [30.0, 40.0, 10.0, 50.0]),
                ("d", [30.0, 90.0, 10.0, 30.0]),
            ],
        ),
        (
            "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-wrap:wrap;max-width:100px;padding:5%",
            &[
                ("c", [270.0, 0.0, 30.0, 140.0]),
                ("a", [280.0, 10.0, 10.0, 40.0]),
                ("b", [280.0, 50.0, 10.0, 50.0]),
                ("d", [280.0, 100.0, 10.0, 30.0]),
            ],
        ),
        (
            "writing-mode:vertical-rl;display:flex;flex-direction:column;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-direction:column;flex-wrap:wrap;max-width:100px;padding:0 10%",
            &[
                ("c", [170.0, 0.0, 130.0, 20.0]),
                ("a", [240.0, 0.0, 40.0, 10.0]),
                ("b", [190.0, 0.0, 50.0, 10.0]),
                ("d", [250.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;min-width:100px;padding:10% 0",
            &[
                ("c", [200.0, 0.0, 100.0, 160.0]),
                ("a", [200.0, 20.0, 100.0, 40.0]),
                ("b", [200.0, 60.0, 100.0, 50.0]),
                ("d", [200.0, 110.0, 100.0, 30.0]),
            ],
        ),
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-rl;display:inline-flex;flex-direction:column;flex-wrap:wrap;max-width:100px;padding:0 10%;box-sizing:border-box",
            &[
                ("c", [0.0, 0.0, 100.0, 30.0]),
                ("a", [30.0, 0.0, 40.0, 10.0]),
                ("b", [20.0, 10.0, 50.0, 10.0]),
                ("d", [40.0, 20.0, 30.0, 10.0]),
            ],
        ),
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-lr;display:inline-flex;padding:5%",
            &[
                ("c", [0.0, 0.0, 40.0, 150.0]),
                ("a", [15.0, 15.0, 10.0, 40.0]),
                ("b", [15.0, 55.0, 10.0, 50.0]),
                ("d", [15.0, 105.0, 10.0, 30.0]),
            ],
        ),
        (
            "display:flex;align-items:flex-start;width:300px;height:200px",
            "writing-mode:vertical-lr;display:inline-flex;flex-direction:column;flex-wrap:wrap;max-width:100px;padding:0 10% 0 0",
            &[
                ("c", [0.0, 0.0, 120.0, 20.0]),
                ("a", [0.0, 0.0, 40.0, 10.0]),
                ("b", [40.0, 0.0, 50.0, 10.0]),
                ("d", [0.0, 10.0, 30.0, 10.0]),
            ],
        ),
    ]);
}

#[test]
fn a_vertical_flex_item_keeps_its_percentage_padding_inside_its_border_box_max_height() {
    // A border-box `max-height` holds the 30px of padding above and below
    // that `padding: 10% 0` resolves to in a 300px wide parent: the box is
    // 100px tall and its single 10px wide column of items does not widen it.
    // How the items break into lines there is another matter, left out.
    assert_cases(&[(
        "display:flex;align-items:flex-start;width:300px;height:200px",
        "writing-mode:vertical-rl;display:inline-flex;flex-wrap:wrap;box-sizing:border-box;max-height:100px;padding:10% 0",
        &[("c", [0.0, 0.0, 30.0, 100.0])],
    )]);
}

#[test]
fn a_vertical_flex_item_adds_its_percentage_padding_to_its_min_height() {
    // In a 200px tall vertical parent, `padding: 5%` is 10px on every side
    // and `padding: 10% 0` 20px above and below; a content-box `min-height`
    // of 150px leaves the items 150px to grow into.
    let parent =
        "writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px";
    assert_cases(&[
        (
            parent,
            "writing-mode:vertical-rl;display:inline-flex;flex-wrap:wrap;min-height:150px;padding:5%",
            &[
                ("c", [270.0, 0.0, 30.0, 170.0]),
                ("a", [280.0, 10.0, 10.0, 55.0]),
                ("b", [280.0, 65.0, 10.0, 65.0]),
                ("d", [280.0, 130.0, 10.0, 30.0]),
            ],
        ),
        (
            parent,
            "writing-mode:vertical-rl;display:flex;flex-wrap:wrap;min-height:150px;padding:10% 0",
            &[
                ("c", [290.0, 0.0, 10.0, 190.0]),
                ("a", [290.0, 20.0, 10.0, 55.0]),
                ("b", [290.0, 75.0, 10.0, 65.0]),
                ("d", [290.0, 140.0, 10.0, 30.0]),
            ],
        ),
    ]);
}
