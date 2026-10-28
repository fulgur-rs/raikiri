//! Flex containers with an `aspect-ratio`: how the content raises the height a
//! container takes from its width, and when it may not.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px: each is the element's
//! left, top, width and height.

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

/// The height a flex container takes from its width through `aspect-ratio` is the least it gets: its content raises it (the automatic minimum size) up to the `max-height` it states, whether the width is its own or the one its parent imposes.
#[test]
fn a_flex_container_ratio_height_is_raised_by_its_content() {
    let cases: &[Case<'_>] = &[
        (
            "a row with an own width and an item taller than the ratio height",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=i style='width:100%;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a column with an own width and an item taller than the ratio height",
            "<div id=c style='display:flex;flex-direction:column;width:100px;aspect-ratio:2/1'><div id=i style='width:100%;height:100px;flex:none'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a max-block-size caps the raise of a row",
            "<div id=c style='display:flex;inline-size:100px;aspect-ratio:2/1;max-block-size:100px'><div id=i style='flex:1;block-size:200px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("i", 8.0, 8.0, 100.0, 200.0)],
        ),
        (
            "a max-block-size caps the raise of a column",
            "<div id=c style='display:flex;flex-direction:column;inline-size:100px;aspect-ratio:2/1;max-block-size:100px'><div id=i style='flex:0 0 200px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("i", 8.0, 8.0, 100.0, 200.0)],
        ),
        (
            "a max-height below the item caps the raise",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1;max-height:60px'><div id=i style='height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 60.0)],
        ),
        (
            "a width the parent imposes",
            "<div id=w style='width:100px'><div id=c style='display:flex;aspect-ratio:2/1'><div id=i style='height:100px'></div></div></div>",
            &[("w", 8.0, 8.0, 100.0, 100.0), ("c", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a percentage width",
            "<div id=w style='width:200px'><div id=c style='display:flex;width:50%;aspect-ratio:2/1'><div id=i style='width:100%;height:100px'></div></div></div>",
            &[
                ("w", 8.0, 8.0, 200.0, 100.0),
                ("c", 8.0, 8.0, 100.0, 100.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
            ],
        ),
        (
            "an inline-flex",
            "<div id=c style='display:inline-flex;width:100px;aspect-ratio:2/1'><div id=i style='height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a float",
            "<div id=c style='float:left;display:flex;width:100px;aspect-ratio:2/1'><div id=i style='width:100%;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "padding adds to the content height",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1;padding:5px'><div id=i style='width:100%;height:100px'></div></div>",
            &[
                ("c", 8.0, 8.0, 100.0, 110.0),
                ("i", 13.0, 13.0, 90.0, 100.0),
            ],
        ),
        (
            "a box that sizes its content box",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1;padding:5px;box-sizing:content-box'><div id=i style='width:100%;height:100px'></div></div>",
            &[
                ("c", 8.0, 8.0, 110.0, 110.0),
                ("i", 13.0, 13.0, 100.0, 100.0),
            ],
        ),
        (
            "wrapped lines add up",
            "<div id=c style='display:flex;flex-wrap:wrap;width:100px;aspect-ratio:2/1'><div id=a style='width:100%;height:40px;flex:none'></div><div id=b style='width:100%;height:40px;flex:none'></div></div>",
            &[
                ("c", 8.0, 8.0, 100.0, 80.0),
                ("a", 8.0, 8.0, 100.0, 40.0),
                ("b", 8.0, 48.0, 100.0, 40.0),
            ],
        ),
        (
            "the raised height is the line the items are centered in",
            "<div id=c style='display:flex;width:100px;aspect-ratio:1/1;align-items:center'><div id=i style='width:100px;height:150px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 150.0), ("i", 8.0, 8.0, 100.0, 150.0)],
        ),
    ];
    check_boxes(cases);
}

/// The ratio height stays when the container scrolls, states a `min-height`, has less content than the ratio height or states a `height`.
#[test]
fn a_flex_container_ratio_height_stays_when_the_content_may_not_raise_it() {
    let cases: &[Case<'_>] = &[
        (
            "overflow: hidden",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1;overflow:hidden'><div id=i style='width:100%;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "min-height: 0",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1;min-height:0'><div id=i style='width:100%;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a min-height below the item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1;min-height:10px'><div id=i style='width:100%;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "less content than the ratio height",
            "<div id=c style='display:flex;width:100px;aspect-ratio:1/1'><div id=i style='height:20px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a height of its own",
            "<div id=c style='display:flex;height:50px;aspect-ratio:2/1'><div id=i style='height:10px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0)],
        ),
    ];
    check_boxes(cases);
}

/// A flex container with a ratio and no width, laid out by a flex row or placed out of flow, takes the width of its content and the height the ratio gives for it, which the content raises unless the container scrolls.
#[test]
fn a_flex_container_with_a_ratio_takes_its_width_from_its_content() {
    let cases: &[Case<'_>] = &[
        (
            "the content is taller than the ratio height",
            "<div id=f style='display:flex;width:300px'><div id=c style='display:flex;aspect-ratio:2/1'><div id=i style='width:60px;height:50px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 300.0, 50.0),
                ("c", 8.0, 8.0, 60.0, 50.0),
                ("i", 8.0, 8.0, 60.0, 50.0),
            ],
        ),
        (
            "the ratio height is the larger",
            "<div id=f style='display:flex;width:300px;align-items:flex-start'><div id=c style='display:flex;aspect-ratio:2/1'><div id=i style='width:60px;height:10px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 300.0, 30.0),
                ("c", 8.0, 8.0, 60.0, 30.0),
                ("i", 8.0, 8.0, 60.0, 10.0),
            ],
        ),
        (
            "overflow: hidden keeps the ratio height",
            "<div id=f style='display:flex;width:300px;align-items:flex-start'><div id=c style='display:flex;aspect-ratio:2/1;overflow:hidden'><div id=i style='width:60px;height:50px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 300.0, 30.0),
                ("c", 8.0, 8.0, 60.0, 30.0),
                ("i", 8.0, 8.0, 60.0, 50.0),
            ],
        ),
        (
            "a width the parent takes from its min-content size",
            "<div id=w style='width:min-content'><div id=c style='display:flex;aspect-ratio:2/1'><div id=i style='width:60px;height:50px'></div></div></div>",
            &[
                ("w", 8.0, 8.0, 60.0, 50.0),
                ("c", 8.0, 8.0, 60.0, 50.0),
                ("i", 8.0, 8.0, 60.0, 50.0),
            ],
        ),
        (
            "a width the parent takes from its max-content size",
            "<div id=w style='width:max-content'><div id=c style='display:flex;aspect-ratio:2/1'><div id=i style='width:60px;height:50px'></div></div></div>",
            &[
                ("w", 8.0, 8.0, 60.0, 50.0),
                ("c", 8.0, 8.0, 60.0, 50.0),
                ("i", 8.0, 8.0, 60.0, 50.0),
            ],
        ),
        (
            "an absolutely positioned container",
            "<div style='position:relative'><div id=c style='position:absolute;display:flex;aspect-ratio:2/1'><div id=i style='width:60px;height:50px'></div></div></div>",
            &[("c", 8.0, 8.0, 60.0, 50.0), ("i", 8.0, 8.0, 60.0, 50.0)],
        ),
    ];
    check_boxes(cases);
}

/// A flex container with a ratio and a height of its own that is an item of a flex column takes its width from that height: the height is the main size the column gives it, and the container derives the width through the ratio when it is not stretched.
#[test]
fn a_flex_container_with_a_ratio_and_a_height_takes_its_width_as_a_column_item() {
    let cases: &[Case<'_>] = &[
        (
            "an empty item aligned to the start",
            "<div id=f style='display:flex;flex-direction:column;width:300px;align-items:flex-start'><div id=c style='display:flex;height:100px;aspect-ratio:2/1'></div></div>",
            &[("f", 8.0, 8.0, 300.0, 100.0), ("c", 8.0, 8.0, 200.0, 100.0)],
        ),
        (
            "an item with content aligned to the start",
            "<div id=f style='display:flex;flex-direction:column;width:300px;align-items:flex-start'><div id=c style='display:flex;height:100px;aspect-ratio:2/1'><div id=i style='width:100%;height:20px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 300.0, 100.0),
                ("c", 8.0, 8.0, 200.0, 100.0),
                ("i", 8.0, 8.0, 200.0, 20.0),
            ],
        ),
        (
            "an item that does not flex",
            "<div id=f style='display:flex;flex-direction:column;width:300px;align-items:flex-start'><div id=c style='display:flex;height:100px;aspect-ratio:2/1;flex:none'><div id=i style='width:100%;height:20px'></div></div></div>",
            &[("c", 8.0, 8.0, 200.0, 100.0), ("i", 8.0, 8.0, 200.0, 20.0)],
        ),
        (
            "a stretched item keeps the width the column gives it",
            "<div id=f style='display:flex;flex-direction:column;width:300px'><div id=c style='display:flex;height:100px;aspect-ratio:2/1'></div></div>",
            &[("c", 8.0, 8.0, 300.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}

/// A flex container with a ratio that is an item of a flex or grid container takes the height the ratio gives for the width the parent settles, which its content raises as for any box.
#[test]
fn a_flex_container_with_a_ratio_is_raised_by_its_content_as_an_item() {
    let cases: &[Case<'_>] = &[
        (
            "an item of a flex row that does not flex",
            "<div id=f style='display:flex;width:300px'><div id=c style='display:flex;width:100px;aspect-ratio:2/1;flex:none'><div id=i style='width:100%;height:100px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 300.0, 100.0),
                ("c", 8.0, 8.0, 100.0, 100.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
            ],
        ),
        (
            "an item of a flex row that stretches",
            "<div id=f style='display:flex;width:300px'><div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=i style='width:100%;height:100px'></div></div></div>",
            &[("f", 8.0, 8.0, 300.0, 100.0), ("c", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "an item of a flex row that is aligned to the start",
            "<div id=f style='display:flex;width:300px;align-items:flex-start'><div id=c style='display:flex;width:100px;aspect-ratio:2/1;flex:none'><div id=i style='width:100%;height:100px'></div></div></div>",
            &[("f", 8.0, 8.0, 300.0, 100.0), ("c", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "an item of a flex row that shrinks",
            "<div id=f style='display:flex;width:50px;align-items:flex-start'><div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=i style='width:100%;height:100px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 50.0, 100.0),
                ("c", 8.0, 8.0, 50.0, 100.0),
                ("i", 8.0, 8.0, 50.0, 100.0),
            ],
        ),
        (
            "an item of a flex column",
            "<div id=f style='display:flex;flex-direction:column;width:300px'><div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=i style='width:100%;height:100px'></div></div></div>",
            &[("f", 8.0, 8.0, 300.0, 100.0), ("c", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "an item of a flex column with less content than the ratio height",
            "<div id=f style='display:flex;flex-direction:column;width:300px'><div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=i style='width:100%;height:20px'></div></div></div>",
            &[("f", 8.0, 8.0, 300.0, 50.0), ("c", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "an item of a flex row that scrolls",
            "<div id=f style='display:flex;width:300px;align-items:flex-start'><div id=c style='display:flex;width:100px;aspect-ratio:2/1;overflow:hidden;flex:none'><div id=i style='width:100%;height:100px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 300.0, 50.0),
                ("c", 8.0, 8.0, 100.0, 50.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
            ],
        ),
        (
            "an item of a grid",
            "<div id=g style='display:grid;width:300px'><div id=c style='display:flex;aspect-ratio:2/1'><div id=i style='width:100%;height:100px'></div></div></div>",
            &[("g", 8.0, 8.0, 300.0, 150.0), ("c", 8.0, 8.0, 300.0, 150.0)],
        ),
        (
            "an item with a ratio of its own that is not stretched",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='display:flex;width:100%;aspect-ratio:1/1;align-self:flex-start'><div id=i style='width:100%;height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}
