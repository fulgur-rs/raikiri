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

/// A `max-width` that clamps the width of a container with a ratio carries through the ratio to a height of the same shape (25px for 50px at 2/1), but the content still raises the height past it, as for a container without the clamp; a container that may not be raised keeps that height.
#[test]
fn a_flex_container_clamped_by_max_width_is_raised_past_the_height_the_clamp_gives() {
    let cases: &[Case<'_>] = &[
        (
            "a row with an own width that max-width clamps",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2;max-width:50px'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0), ("i", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "a column with an auto width that max-width clamps",
            "<div id=c style='display:flex;flex-direction:column;aspect-ratio:2;max-width:50px'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a max-height still caps the raise",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2;max-width:50px;max-height:60px'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 60.0), ("i", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "min-height: 0 keeps the clamped ratio height",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2;max-width:50px;min-height:0'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 25.0)],
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

/// The items a single-line flex row stretches fill the line, so neither their content nor their own aspect ratio raises the container past the height its ratio gives it, whichever kind of box the item is and wherever the container is laid out; the line is as high as the ratio height says.
#[test]
fn a_row_container_ratio_height_is_not_raised_by_the_items_it_stretches() {
    let cases: &[Case<'_>] = &[
        (
            "a stretched item with a ratio",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a stretched item without a ratio and with taller content",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%'><div style='height:80px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a stretched item with a ratio and much taller content",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:200px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a stretched item with a ratio next to a short item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:50px;aspect-ratio:1/1'><div style='height:80px'></div></div><div id=e style='width:50px;height:30px'></div></div>",
            &[
                ("c", 8.0, 8.0, 100.0, 50.0),
                ("d", 8.0, 8.0, 50.0, 50.0),
                ("e", 58.0, 8.0, 50.0, 30.0),
            ],
        ),
        (
            "a stretched item with a tall ratio",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/2'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "the padding of a stretched item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1;padding:5px 0'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "the border of a stretched item that sizes its content box",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;border:3px solid;box-sizing:content-box'><div style='height:80px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "the margins of a stretched item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1;margin:5px 0'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 13.0, 100.0, 40.0)],
        ),
        (
            "align-self: stretch over another align-items",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1;align-items:flex-end'><div id=d style='width:100%;aspect-ratio:1/1;align-self:stretch'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a flex basis of 100%",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1;height:auto;flex-basis:100%'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "an item that grows",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='flex:1 1 0;aspect-ratio:1/1'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a paragraph item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'>AAAA</div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a flex container item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='display:flex;width:100%;aspect-ratio:1/1'><div style='height:30px;width:10px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a grid container item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='display:grid;width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "an inline-flex container",
            "<div id=c style='display:inline-flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a width the parent imposes",
            "<div style='width:100px'><div id=c style='display:flex;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "an item of a flex row",
            "<div style='display:flex;width:100px'><div id=c style='display:flex;aspect-ratio:2/1;flex:1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "an item of a grid",
            "<div style='display:grid;width:100px'><div id=c style='display:flex;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a float",
            "<div style='float:left'><div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "an inline-flex container that takes its width from its content",
            "<div id=c style='display:inline-flex;aspect-ratio:2/1'><div id=d style='width:100px;aspect-ratio:1/1'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a maximum height on the stretched item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1;max-height:20px'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0)],
        ),
    ];
    check_boxes(cases);
}

/// What raises a flex row with a ratio is what its line does not stretch: an item that states a height, or aligns to something else than the line, or has auto margins, and the `min-height`, padding and margins of a stretched one; a container that wraps its items or lays them out in a grid has no line of the height of its ratio.
#[test]
fn a_row_container_ratio_height_is_raised_by_what_the_line_does_not_stretch() {
    let cases: &[Case<'_>] = &[
        (
            "the min-height of a stretched item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1;min-height:60px'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 60.0), ("d", 8.0, 8.0, 100.0, 60.0)],
        ),
        (
            "the min-height of a stretched item that sizes its content box",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;min-height:40px;padding:10px 0;box-sizing:content-box'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 60.0), ("d", 8.0, 8.0, 100.0, 60.0)],
        ),
        (
            "the min-height and margins of a stretched item",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;min-height:20px;margin:30px 0'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 80.0), ("d", 8.0, 38.0, 100.0, 20.0)],
        ),
        (
            "padding taller than the ratio height",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;padding:40px 0;box-sizing:border-box'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 80.0), ("d", 8.0, 8.0, 100.0, 80.0)],
        ),
        (
            "a centered item taller than the ratio height",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:50px'><div style='height:80px'></div></div><div id=e style='width:50px;align-self:center;height:150px'></div></div>",
            &[
                ("c", 8.0, 8.0, 100.0, 150.0),
                ("d", 8.0, 8.0, 50.0, 150.0),
                ("e", 58.0, 8.0, 50.0, 150.0),
            ],
        ),
        (
            "an item at the end with a margin",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:50px'><div style='height:30px'></div></div><div id=e style='width:50px;align-self:flex-end;height:80px;margin-top:10px'></div></div>",
            &[
                ("c", 8.0, 8.0, 100.0, 90.0),
                ("d", 8.0, 8.0, 50.0, 90.0),
                ("e", 58.0, 18.0, 50.0, 80.0),
            ],
        ),
        (
            "an item aligned to the baseline",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1;align-items:baseline'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "an item with an auto margin",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1;margin-top:auto'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "items aligned to the start",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1;align-items:flex-start'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a tall item next to a stretched one",
            "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:50%;align-self:flex-start'><div style='height:80px'></div></div><div id=e style='width:50%;aspect-ratio:1/1;'><div style='height:10px'></div></div></div>",
            &[
                ("c", 8.0, 8.0, 100.0, 80.0),
                ("d", 8.0, 8.0, 50.0, 80.0),
                ("e", 58.0, 8.0, 50.0, 80.0),
            ],
        ),
        (
            "a container that wraps its items",
            "<div id=c style='display:flex;flex-wrap:wrap;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a grid container",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}

/// A container that is the item of a flex column finds its height from its content first, so the items it stretches count at their own heights there, whether the container states its width or is stretched.
#[test]
fn a_ratio_container_in_a_flex_column_keeps_the_heights_of_its_items() {
    let cases: &[Case<'_>] = &[
        (
            "a stretched container",
            "<div style='display:flex;flex-direction:column;width:100px'><div id=c style='display:flex;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a container that states its width",
            "<div style='display:flex;flex-direction:column;align-items:flex-start'><div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a container that states its width in a wider column",
            "<div style='display:flex;flex-direction:column;align-items:flex-start;width:200px'><div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a container with a plain item",
            "<div style='display:flex;flex-direction:column;width:100px'><div id=c style='display:flex;aspect-ratio:2/1'><div id=d style='width:100%'><div style='height:80px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 80.0), ("d", 8.0, 8.0, 100.0, 80.0)],
        ),
        (
            "a container whose ratio height is the larger",
            "<div style='display:flex;flex-direction:column;width:300px'><div id=c style='display:flex;aspect-ratio:2/1;flex:none'><div id=d style='width:100px;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 300.0, 150.0), ("d", 8.0, 8.0, 100.0, 150.0)],
        ),
        (
            "a container that grows",
            "<div style='display:flex;flex-direction:column;width:300px;height:400px'><div id=c style='display:flex;aspect-ratio:2/1;flex:1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 300.0, 400.0), ("d", 8.0, 8.0, 300.0, 400.0)],
        ),
        (
            "a reversed column",
            "<div style='display:flex;flex-direction:column-reverse;width:100px'><div id=c style='display:flex;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1'><div style='height:30px'></div></div></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}

#[test]
fn a_percentage_minimum_raises_the_ratio_height_of_a_flex_container() {
    check_boxes(&[(
        "ratio height resolves the item percentage minimum",
        "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;min-height:120%'><div style='height:10px'></div></div></div>",
        &[("c", 8.0, 8.0, 100.0, 60.0), ("d", 8.0, 8.0, 100.0, 60.0)],
    )]);
}

#[test]
fn a_percentage_height_raises_the_ratio_height_of_a_flex_container() {
    check_boxes(&[(
        "ratio height resolves the item percentage height",
        "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;height:150%'><div style='height:10px'></div></div></div>",
        &[("c", 8.0, 8.0, 100.0, 75.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn a_calc_percentage_height_keeps_the_item_below_its_containers_ratio_height() {
    check_boxes(&[(
        "ratio height resolves the item calc percentage",
        "<div id=c style='display:flex;width:100px;aspect-ratio:2/1'><div id=d style='width:100%;aspect-ratio:1/1;height:calc(100% - 10px)'><div style='height:30px'></div></div></div>",
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 40.0)],
    )]);
}

#[test]
fn a_percentage_height_in_a_ratio_column_resolves_before_its_item_ratio() {
    check_boxes(&[(
        "ratio column height resolves before item ratio",
        "<div id=c style='display:flex;flex-direction:column;width:100px;aspect-ratio:2/1'><div id=d style='height:100%;aspect-ratio:1/1'><div style='width:30px;height:10px'></div></div></div>",
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_row_minimum() {
    check_boxes(&[(
        "row-minimum",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 60.0), ("d", 8.0, 8.0, 100.0, 60.0)],
    )]);
}

#[test]
fn ratio_percentage_row_minimum_nested_column() {
    check_boxes(&[(
        "row-minimum-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 60.0)],
    )]);
}

#[test]
fn ratio_percentage_row_height() {
    check_boxes(&[(
        "row-height",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 75.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_row_height_nested_column() {
    check_boxes(&[(
        "row-height-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:150%"><div style="width:30px;height:10px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_row_calc() {
    check_boxes(&[(
        "row-calc",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;height:calc(100% - 10px)"><div style="width:30px;height:30px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 40.0)],
    )]);
}

#[test]
fn ratio_percentage_row_calc_nested_column() {
    check_boxes(&[(
        "row-calc-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;height:calc(100% - 10px)"><div style="width:30px;height:30px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 90.0)],
    )]);
}

#[test]
fn ratio_percentage_row_column_original() {
    check_boxes(&[(
        "row-column-original",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="height:100%;aspect-ratio:1/1"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 50.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_row_column_original_nested_column() {
    check_boxes(&[(
        "row-column-original-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="height:100%;aspect-ratio:1/1"><div style="width:30px;height:10px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 50.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_row_max() {
    check_boxes(&[(
        "row-max",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;max-height:50%;aspect-ratio:1/1"><div style="width:30px;height:80px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 25.0)],
    )]);
}

#[test]
fn ratio_percentage_row_max_nested_column() {
    check_boxes(&[(
        "row-max-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;max-height:50%;aspect-ratio:1/1"><div style="width:30px;height:80px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_row_percentage() {
    check_boxes(&[(
        "row-percentage",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:80%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 40.0)],
    )]);
}

#[test]
fn ratio_percentage_row_percentage_nested_column() {
    check_boxes(&[(
        "row-percentage-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:80%"><div style="width:30px;height:10px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 40.0)],
    )]);
}

#[test]
fn ratio_percentage_column_minimum() {
    check_boxes(&[(
        "column-minimum",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 60.0), ("d", 8.0, 8.0, 100.0, 60.0)],
    )]);
}

#[test]
fn ratio_percentage_column_minimum_nested_column() {
    check_boxes(&[(
        "column-minimum-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 60.0)],
    )]);
}

#[test]
fn ratio_percentage_column_height() {
    check_boxes(&[(
        "column-height",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 75.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_column_height_nested_column() {
    check_boxes(&[(
        "column-height-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:150%"><div style="width:30px;height:10px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_column_calc() {
    check_boxes(&[(
        "column-calc",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;height:calc(100% - 10px)"><div style="width:30px;height:30px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 40.0)],
    )]);
}

#[test]
fn ratio_percentage_column_calc_nested_column() {
    check_boxes(&[(
        "column-calc-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;aspect-ratio:1/1;height:calc(100% - 10px)"><div style="width:30px;height:30px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 90.0)],
    )]);
}

#[test]
fn ratio_percentage_column_column_original() {
    check_boxes(&[(
        "column-column-original",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="height:100%;aspect-ratio:1/1"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_column_column_original_nested_column() {
    check_boxes(&[(
        "column-column-original-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="height:100%;aspect-ratio:1/1"><div style="width:30px;height:10px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 100.0)],
    )]);
}

#[test]
fn ratio_percentage_column_max() {
    check_boxes(&[(
        "column-max",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;max-height:50%;aspect-ratio:1/1"><div style="width:30px;height:80px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 25.0)],
    )]);
}

#[test]
fn ratio_percentage_column_max_nested_column() {
    check_boxes(&[(
        "column-max-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;max-height:50%;aspect-ratio:1/1"><div style="width:30px;height:80px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 100.0), ("d", 8.0, 8.0, 100.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_column_percentage() {
    check_boxes(&[(
        "column-percentage",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:80%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 40.0)],
    )]);
}

#[test]
fn ratio_percentage_column_percentage_nested_column() {
    check_boxes(&[(
        "column-percentage-nested-column",
        r#"<div style="display:flex;flex-direction:column;width:100px"><div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:80%"><div style="width:30px;height:10px"></div></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 40.0)],
    )]);
}

#[test]
fn ratio_percentage_row_border_box_edges() {
    check_boxes(&[(
        "row-border-box-edges",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1;padding:10px;border:5px solid"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 54.0), ("d", 23.0, 23.0, 70.0, 24.0)],
    )]);
}

#[test]
fn ratio_percentage_row_content_box_edges() {
    check_boxes(&[(
        "row-content-box-edges",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1;box-sizing:content-box;padding:10px;border:5px solid"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 130.0, 90.0), ("d", 23.0, 23.0, 100.0, 60.0)],
    )]);
}

#[test]
fn ratio_percentage_row_max_height() {
    check_boxes(&[(
        "row-max-height",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1;max-height:40px"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 40.0), ("d", 8.0, 8.0, 100.0, 48.0)],
    )]);
}

#[test]
fn ratio_percentage_row_no_auto_min() {
    check_boxes(&[(
        "row-no-auto-min",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1;min-height:0"><div id=d style="width:100%;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_row_wrapped() {
    check_boxes(&[(
        "row-wrapped",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div id=d style="width:100%;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 75.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_row_centered() {
    check_boxes(&[(
        "row-centered",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1;align-items:center"><div id=d style="width:100%;aspect-ratio:1/1;height:calc(100% - 10px)"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 13.0, 100.0, 40.0)],
    )]);
}

#[test]
fn ratio_percentage_row_vertical_item() {
    check_boxes(&[(
        "row-vertical-item",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1;"><div id=d style="width:100%;writing-mode:vertical-lr;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 75.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_row_absolute_item() {
    check_boxes(&[(
        "row-absolute-item",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1;position:relative"><div id=d style="width:100%;position:absolute;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_row_math_min() {
    check_boxes(&[(
        "row-math-min",
        r#"<div id=c style="display:flex;flex-direction:row;width:100px;aspect-ratio:2/1;"><div id=d style="width:100%;min-height:min(120%, 80px)"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 60.0), ("d", 8.0, 8.0, 100.0, 60.0)],
    )]);
}

#[test]
fn ratio_percentage_column_border_box_edges() {
    check_boxes(&[(
        "column-border-box-edges",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1;padding:10px;border:5px solid"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 54.0), ("d", 23.0, 23.0, 70.0, 24.0)],
    )]);
}

#[test]
fn ratio_percentage_column_content_box_edges() {
    check_boxes(&[(
        "column-content-box-edges",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1;box-sizing:content-box;padding:10px;border:5px solid"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 130.0, 90.0), ("d", 23.0, 23.0, 100.0, 60.0)],
    )]);
}

#[test]
fn ratio_percentage_column_max_height() {
    check_boxes(&[(
        "column-max-height",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1;max-height:40px"><div id=d style="width:100%;min-height:120%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 40.0), ("d", 8.0, 8.0, 100.0, 48.0)],
    )]);
}

#[test]
fn ratio_percentage_column_no_auto_min() {
    check_boxes(&[(
        "column-no-auto-min",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1;min-height:0"><div id=d style="width:100%;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_column_wrapped() {
    check_boxes(&[(
        "column-wrapped",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div id=d style="width:100%;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 75.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_column_centered() {
    check_boxes(&[(
        "column-centered",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1;align-items:center"><div id=d style="width:100%;aspect-ratio:1/1;height:calc(100% - 10px)"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 40.0)],
    )]);
}

#[test]
fn ratio_percentage_column_vertical_item() {
    check_boxes(&[(
        "column-vertical-item",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1;"><div id=d style="width:100%;writing-mode:vertical-lr;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 75.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_column_absolute_item() {
    check_boxes(&[(
        "column-absolute-item",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1;position:relative"><div id=d style="width:100%;position:absolute;height:150%"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_column_math_min() {
    check_boxes(&[(
        "column-math-min",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1;"><div id=d style="width:100%;min-height:min(120%, 80px)"><div style="width:30px;height:10px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 60.0), ("d", 8.0, 8.0, 100.0, 60.0)],
    )]);
}

#[test]
fn hidden_percentage_items_do_not_raise_the_ratio_height_or_produce_fragments() {
    for direction in ["row", "column"] {
        let body = format!(
            "<div id=c style='display:flex;flex-direction:{direction};width:100px;aspect-ratio:2/1'><div id=d style='display:none;height:150%'></div></div>"
        );
        let actual = boxes(&body);
        assert_eq!(
            actual.iter().find(|(id, _)| id == "c").unwrap().1,
            [8.0, 8.0, 100.0, 50.0]
        );
        assert!(actual.iter().all(|(id, _)| id != "d"));
    }
}

#[test]
fn ratio_percentage_column_basis_50_height_auto() {
    check_boxes(&[(
        "column-basis-50-height-auto",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:auto;flex:0 0 50%"><div style="height:10px;width:30px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 25.0)],
    )]);
}

#[test]
fn ratio_percentage_column_basis_50_height_150percent() {
    check_boxes(&[(
        "column-basis-50-height-150percent",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:150%;flex:0 0 50%"><div style="height:10px;width:30px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 25.0)],
    )]);
}

#[test]
fn ratio_percentage_column_basis_100_height_auto() {
    check_boxes(&[(
        "column-basis-100-height-auto",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:auto;flex:0 0 100%"><div style="height:10px;width:30px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_column_basis_100_height_150percent() {
    check_boxes(&[(
        "column-basis-100-height-150percent",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:150%;flex:0 0 100%"><div style="height:10px;width:30px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 50.0), ("d", 8.0, 8.0, 100.0, 50.0)],
    )]);
}

#[test]
fn ratio_percentage_column_basis_150_height_auto() {
    check_boxes(&[(
        "column-basis-150-height-auto",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:auto;flex:0 0 150%"><div style="height:10px;width:30px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 75.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}

#[test]
fn ratio_percentage_column_basis_150_height_150percent() {
    check_boxes(&[(
        "column-basis-150-height-150percent",
        r#"<div id=c style="display:flex;flex-direction:column;width:100px;aspect-ratio:2/1"><div id=d style="width:100%;height:150%;flex:0 0 150%"><div style="height:10px;width:30px"></div></div></div>"#,
        &[("c", 8.0, 8.0, 100.0, 75.0), ("d", 8.0, 8.0, 100.0, 75.0)],
    )]);
}
