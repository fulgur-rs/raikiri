//! Grid containers and grid items with an `aspect-ratio`: which alignment
//! sizes an item that has the ratio, how the content raises the height a
//! container takes from its width, and how a percentage size takes part in
//! measuring a ratio item.
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

/// A ratio item with `auto` sizes in a grid area takes the height of the area when its block-axis alignment is an explicit `stretch` and its inline-axis alignment is `normal`, and the width the ratio gives for that height. `align-items: stretch` on the container counts as explicit.
#[test]
fn an_explicit_stretch_in_the_block_axis_beats_normal_alignment_in_the_inline_axis() {
    let cases: &[Case<'_>] = &[
        (
            "align-self: stretch, a wider column",
            "<div id=g style='display:grid;grid-template:100px/200px'><div id=a style='aspect-ratio:1/1;align-self:stretch'></div></div>",
            &[("a", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "align-self: stretch, a narrower column",
            "<div id=g style='display:grid;grid-template:100px/50px'><div id=a style='aspect-ratio:1/1;align-self:stretch'></div></div>",
            &[("a", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "align-items: stretch on the container",
            "<div id=g style='display:grid;grid-template:100px/200px;align-items:stretch'><div id=a style='aspect-ratio:1/1'></div></div>",
            &[("a", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "another ratio",
            "<div id=g style='display:grid;grid-template:100px/200px'><div id=a style='aspect-ratio:2/1;align-self:stretch'></div></div>",
            &[("a", 8.0, 8.0, 200.0, 100.0)],
        ),
        (
            "a width of its own",
            "<div id=g style='display:grid;grid-template:100px/200px'><div id=a style='aspect-ratio:1/1;align-self:stretch;width:20px'></div></div>",
            &[("a", 8.0, 8.0, 20.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}

/// Without an explicit block-axis `stretch` the width is stretched and the height follows from it; an explicit stretch in both axes ignores the ratio, and an `auto` margin turns the block-axis stretch off.
#[test]
fn normal_alignment_alone_still_stretches_the_width_of_a_ratio_item() {
    let cases: &[Case<'_>] = &[
        (
            "no alignment at all",
            "<div id=g style='display:grid;grid-template:100px/200px'><div id=a style='aspect-ratio:1/1'></div></div>",
            &[("a", 8.0, 8.0, 200.0, 200.0)],
        ),
        (
            "justify-self: stretch",
            "<div id=g style='display:grid;grid-template:100px/200px'><div id=a style='aspect-ratio:1/1;justify-self:stretch'></div></div>",
            &[("a", 8.0, 8.0, 200.0, 200.0)],
        ),
        (
            "a stretch in both axes",
            "<div id=g style='display:grid;grid-template:100px/200px'><div id=a style='aspect-ratio:1/1;align-self:stretch;justify-self:stretch'></div></div>",
            &[("a", 8.0, 8.0, 200.0, 100.0)],
        ),
        (
            "an auto margin in the block axis",
            "<div id=g style='display:grid;grid-template:100px/200px'><div id=a style='aspect-ratio:1/1;align-self:stretch;margin-top:auto'></div></div>",
            &[("a", 8.0, 8.0, 200.0, 200.0)],
        ),
    ];
    check_boxes(cases);
}

/// The height a grid container takes from its width through `aspect-ratio` is the least it gets: its content raises it (the automatic minimum size) up to the `max-height` it states, whether the width is its own or the one its parent imposes.
#[test]
fn a_grid_container_ratio_height_is_raised_by_its_content() {
    let cases: &[Case<'_>] = &[
        (
            "an own width and an item taller than the ratio height",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2/1'><div id=i style='width:100%;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a max-block-size caps the raise",
            "<div id=c style='display:grid;inline-size:100px;aspect-ratio:2/1;max-block-size:100px'><div id=i style='inline-size:100%;block-size:200px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("i", 8.0, 8.0, 100.0, 200.0)],
        ),
        (
            "a max-height below the item caps the raise",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2/1;max-height:60px'><div id=i style='height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 60.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a width the parent imposes, items centered in the block axis",
            "<div id=w style='width:100px'><div id=c style='display:grid;aspect-ratio:1/1;align-items:center'><div id=a style='width:100px;height:50px'></div></div></div>",
            &[
                ("w", 8.0, 8.0, 100.0, 100.0),
                ("c", 8.0, 8.0, 100.0, 100.0),
                ("a", 8.0, 33.0, 100.0, 50.0),
            ],
        ),
        (
            "a width the parent imposes",
            "<div id=w style='width:100px'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='height:100px'></div></div></div>",
            &[
                ("w", 8.0, 8.0, 100.0, 100.0),
                ("c", 8.0, 8.0, 100.0, 100.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
            ],
        ),
        (
            "an inline-grid",
            "<div id=c style='display:inline-grid;width:100px;aspect-ratio:2/1'><div id=i style='height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}

/// A `max-width` that clamps the width of a container with a ratio carries through the ratio to a height of the same shape (25px for 50px at 2/1), but the content still raises the height past it, as for a container without the clamp; a container that may not be raised keeps that height.
#[test]
fn a_grid_container_clamped_by_max_width_is_raised_past_the_height_the_clamp_gives() {
    let cases: &[Case<'_>] = &[
        (
            "an own width that max-width clamps, content centered in the block axis",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2;max-width:50px;align-content:center'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "an own width that max-width clamps",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2;max-width:50px'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "an auto width that max-width clamps",
            "<div id=c style='display:grid;aspect-ratio:2;max-width:50px'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "an inline-grid",
            "<div id=c style='display:inline-grid;width:100px;aspect-ratio:2;max-width:50px'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a width the parent imposes and max-width clamps",
            "<div id=w style='width:100px'><div id=c style='display:grid;aspect-ratio:2;max-width:50px'><div id=i style='width:100px;height:100px'></div></div></div>",
            &[
                ("w", 8.0, 8.0, 100.0, 100.0),
                ("c", 8.0, 8.0, 50.0, 100.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
            ],
        ),
        (
            "an item of a flex row",
            "<div id=w style='display:flex'><div id=c style='display:grid;width:100px;aspect-ratio:2;max-width:50px'><div id=i style='width:100px;height:100px'></div></div></div>",
            &[
                ("w", 8.0, 8.0, 784.0, 100.0),
                ("c", 8.0, 8.0, 50.0, 100.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
            ],
        ),
        (
            "a min-width above the max-width wins, and so does the content",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2;min-width:80px;max-width:50px'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 80.0, 100.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a max-height still caps the raise",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2;max-width:50px;max-height:60px'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 60.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "overflow: hidden keeps the clamped ratio height",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2;max-width:50px;overflow:hidden'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 25.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "min-height: 0 keeps the clamped ratio height",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2;max-width:50px;min-height:0'><div id=i style='width:100px;height:100px'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 25.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}

/// The ratio height stays when the container scrolls, states a `min-height` or has less content than the ratio height, and a container sized by its height takes its width from it.
#[test]
fn a_grid_container_ratio_height_stays_when_the_content_may_not_raise_it() {
    let cases: &[Case<'_>] = &[
        (
            "overflow: hidden",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2/1;overflow:hidden'><div id=i style='height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "min-height: 0",
            "<div id=c style='display:grid;width:100px;aspect-ratio:2/1;min-height:0'><div id=i style='height:100px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("i", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "less content than the ratio height",
            "<div id=c style='display:grid;width:100px;aspect-ratio:1/1'><div id=i style='height:20px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0), ("i", 8.0, 8.0, 100.0, 20.0)],
        ),
        (
            "a height of its own",
            "<div id=c style='display:grid;height:50px;aspect-ratio:2/1'><div id=i style='height:10px'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0), ("i", 8.0, 8.0, 100.0, 10.0)],
        ),
        (
            "overflow: hidden under a parent-imposed width",
            "<div id=w style='width:100px'><div id=c style='display:grid;aspect-ratio:2/1;overflow:hidden'><div id=i style='height:100px'></div></div></div>",
            &[
                ("w", 8.0, 8.0, 100.0, 50.0),
                ("c", 8.0, 8.0, 100.0, 50.0),
                ("i", 8.0, 8.0, 100.0, 100.0),
            ],
        ),
        (
            "a flexible row fills the ratio height",
            "<div id=w style='width:100px'><div id=c style='display:grid;aspect-ratio:2/1;grid-template-rows:1fr'><div id=i></div></div></div>",
            &[
                ("w", 8.0, 8.0, 100.0, 50.0),
                ("c", 8.0, 8.0, 100.0, 50.0),
                ("i", 8.0, 8.0, 100.0, 50.0),
            ],
        ),
    ];
    check_boxes(cases);
}

/// A percentage `width` or `height` of a grid item behaves as `auto` while the track it sits in is sized, so the ratio gives the item the size the track grows to; the percentage then resolves against that track.
#[test]
fn a_percentage_size_is_cyclic_while_a_grid_measures_a_ratio_item() {
    let cases: &[Case<'_>] = &[
        (
            "a percentage width, the grid shrinks to the item",
            "<div id=g style='display:grid;width:min-content;height:100px'><div id=a style='height:100px;width:50%;aspect-ratio:1/1'></div></div>",
            &[("g", 8.0, 8.0, 100.0, 100.0), ("a", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "a percentage height, the grid shrinks to the item",
            "<div id=g style='display:grid;height:min-content;width:100px'><div id=a style='width:100px;height:50%;aspect-ratio:1/1'></div></div>",
            &[("g", 8.0, 8.0, 100.0, 100.0), ("a", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "percentage sizes in both axes",
            "<style>.grid{display:grid;width:100px}.item{all:unset;aspect-ratio:1/1;background-color:green;height:100%;width:100%}</style><div class=grid id=g><div id=a class=item></div></div>",
            &[("g", 8.0, 8.0, 100.0, 100.0), ("a", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a percentage width with a min-width: min-content",
            "<div id=g style='display:grid;grid-template-columns:auto;width:min-content'><div id=a style='height:50px;width:min-content;aspect-ratio:2/1'></div><div id=b style='height:50px;width:1%;min-width:min-content;aspect-ratio:2/1'></div></div>",
            &[
                ("g", 8.0, 8.0, 100.0, 100.0),
                ("a", 8.0, 8.0, 100.0, 50.0),
                ("b", 8.0, 58.0, 100.0, 50.0),
            ],
        ),
        (
            "calc() with a percentage",
            "<div id=g style='display:inline-grid'><div id=a style='height:100px;width:calc(50% + 0px);aspect-ratio:1/1'></div></div>",
            &[("g", 8.0, 8.0, 100.0, 100.0), ("a", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "calc() with a percentage and a length",
            "<div id=g style='display:inline-grid'><div id=a style='height:100px;width:calc(50% + 20px);aspect-ratio:1/1'></div></div>",
            &[("g", 8.0, 8.0, 100.0, 100.0), ("a", 8.0, 8.0, 70.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}

/// The same holds for an inline-block, an inline-flex and an inline-grid, whose width comes from the item.
#[test]
fn a_percentage_size_is_cyclic_in_a_shrink_to_fit_box() {
    let cases: &[Case<'_>] = &[
        (
            "an inline-block",
            "<div id=g style='display:inline-block'><div id=a style='height:100px;width:50%;aspect-ratio:1/1'></div></div>",
            &[("g", 8.0, 8.0, 100.0, 100.0), ("a", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "an inline-flex",
            "<div id=g style='display:inline-flex'><div id=a style='height:100px;width:50%;aspect-ratio:1/1'></div></div>",
            &[("g", 8.0, 8.0, 100.0, 100.0), ("a", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "an inline-grid",
            "<div id=g style='display:inline-grid'><div id=a style='height:100px;width:50%;aspect-ratio:1/1'></div></div>",
            &[("g", 8.0, 8.0, 100.0, 100.0), ("a", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "a percentage height in an inline-block",
            "<div id=g style='display:inline-block'><div id=a style='width:100px;height:50%;aspect-ratio:1/1'></div></div>",
            &[("g", 8.0, 8.0, 100.0, 100.0), ("a", 8.0, 8.0, 100.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}

/// A grid container with a ratio and no width, laid out by a flex row, takes the width of its content and the height the ratio gives for it, which the content raises unless the container scrolls.
#[test]
fn a_grid_container_with_a_ratio_in_a_flex_row_takes_its_width_from_its_content() {
    let cases: &[Case<'_>] = &[
        (
            "the ratio height is the larger",
            "<div id=f style='display:flex;width:300px'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:10px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 300.0, 30.0),
                ("c", 8.0, 8.0, 60.0, 30.0),
                ("i", 8.0, 8.0, 60.0, 10.0),
            ],
        ),
        (
            "the content is taller than the ratio height",
            "<div id=f style='display:flex;width:300px'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:50px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 300.0, 50.0),
                ("c", 8.0, 8.0, 60.0, 50.0),
                ("i", 8.0, 8.0, 60.0, 50.0),
            ],
        ),
        (
            "overflow: hidden keeps the ratio height",
            "<div id=f style='display:flex;width:300px;align-items:flex-start'><div id=c style='display:grid;aspect-ratio:2/1;overflow:hidden'><div id=i style='width:60px;height:50px'></div></div></div>",
            &[
                ("f", 8.0, 8.0, 300.0, 30.0),
                ("c", 8.0, 8.0, 60.0, 30.0),
                ("i", 8.0, 8.0, 60.0, 50.0),
            ],
        ),
    ];
    check_boxes(cases);
}

/// A grid container with a ratio whose width comes from its content, where
/// no width is known when it is first asked for its height, takes the width
/// of the content and the height the ratio gives for it.
#[test]
fn a_grid_container_with_a_ratio_settles_a_width_that_comes_from_its_content() {
    let cases: &[Case<'_>] = &[
        (
            "an absolutely positioned container",
            "<div style='position:relative'><div id=c style='position:absolute;display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:10px'></div></div></div>",
            &[("c", 8.0, 8.0, 60.0, 30.0), ("i", 8.0, 8.0, 60.0, 10.0)],
        ),
        (
            "an absolutely positioned container whose content is taller",
            "<div style='position:relative'><div id=c style='position:absolute;display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:50px'></div></div></div>",
            &[("c", 8.0, 8.0, 60.0, 50.0), ("i", 8.0, 8.0, 60.0, 50.0)],
        ),
        (
            "a column flex item aligned at the start",
            "<div style='display:flex;flex-direction:column;align-items:flex-start'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:10px'></div></div></div>",
            &[("c", 8.0, 8.0, 60.0, 30.0), ("i", 8.0, 8.0, 60.0, 10.0)],
        ),
        (
            "a grid item aligned at the start",
            "<div style='display:grid;justify-items:start'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:10px'></div></div></div>",
            &[("c", 8.0, 8.0, 60.0, 30.0), ("i", 8.0, 8.0, 60.0, 10.0)],
        ),
        (
            "a box that is as narrow as its content",
            "<div style='width:min-content'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:10px'></div></div></div>",
            &[("c", 8.0, 8.0, 60.0, 30.0), ("i", 8.0, 8.0, 60.0, 10.0)],
        ),
        (
            "a table cell",
            "<table style='border-spacing:0'><tr><td style='padding:0'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:10px'></div></div></td></tr></table>",
            &[("c", 8.0, 8.0, 60.0, 30.0), ("i", 8.0, 8.0, 60.0, 10.0)],
        ),
        (
            "an inline-block parent",
            "<div id=p style='display:inline-block'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:10px'></div></div></div>",
            &[
                ("p", 8.0, 8.0, 60.0, 30.0),
                ("c", 8.0, 8.0, 60.0, 30.0),
                ("i", 8.0, 8.0, 60.0, 10.0),
            ],
        ),
        (
            "an inline-block parent with taller content",
            "<div id=p style='display:inline-block'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:50px'></div></div></div>",
            &[
                ("p", 8.0, 8.0, 60.0, 50.0),
                ("c", 8.0, 8.0, 60.0, 50.0),
                ("i", 8.0, 8.0, 60.0, 50.0),
            ],
        ),
        (
            "a float parent",
            "<div id=p style='float:left'><div id=c style='display:grid;aspect-ratio:2/1'><div id=i style='width:60px;height:10px'></div></div></div>",
            &[
                ("p", 8.0, 8.0, 60.0, 30.0),
                ("c", 8.0, 8.0, 60.0, 30.0),
                ("i", 8.0, 8.0, 60.0, 10.0),
            ],
        ),
    ];
    check_boxes(cases);
}

/// A percentage size that resolves leaves the ratio nothing to do, and the
/// ratio does not carry a `min-*` or `max-*` size across to the other axis
/// either: both sizes the box states win over the bound.
#[test]
fn a_percentage_that_resolves_keeps_the_bounds_from_crossing_the_ratio() {
    let cases: &[Case<'_>] = &[
        (
            "a grid container, `max-width` clamps the percentage width",
            "<div style='width:200px'><div id=c style='display:grid;width:50%;height:100px;max-width:50px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "a grid container, `min-width` raises the percentage width",
            "<div style='width:200px'><div id=c style='display:grid;width:50%;height:100px;min-width:150px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 150.0, 100.0)],
        ),
        (
            "a grid container, `max-height` clamps the height",
            "<div style='width:200px'><div id=c style='display:grid;width:50%;height:100px;max-height:30px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 30.0)],
        ),
        (
            "a block container",
            "<div style='width:200px'><div id=c style='width:50%;height:100px;max-width:50px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "a flex container",
            "<div style='width:200px'><div id=c style='display:flex;width:50%;height:100px;max-width:50px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "a block item in a grid area",
            "<div style='display:grid;grid-template-columns:200px;grid-template-rows:100px'><div id=c style='width:50%;height:100px;max-width:50px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 100.0)],
        ),
        (
            "percentage sizes of a grid item",
            "<div style='display:grid;grid-template-columns:200px;grid-template-rows:100px'><div id=c style='width:50%;height:50%;max-width:50px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 50.0, 50.0)],
        ),
        (
            "a grid item, `min-width` raises the width",
            "<div style='display:grid;grid-template-columns:200px;grid-template-rows:100px'><div id=c style='width:50%;height:100px;min-width:150px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 150.0, 100.0)],
        ),
        (
            "a grid container with a smaller `max-height`",
            "<div style='width:200px'><div id=c style='display:grid;width:50%;height:50px;max-height:30px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 30.0)],
        ),
        (
            "percentage sizes against a box with a height, `max-width`",
            "<div style='height:100px;width:200px'><div id=c style='display:grid;width:50%;height:50%;max-width:30px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 30.0, 50.0)],
        ),
        (
            "percentage sizes against a box with a height, `min-width`",
            "<div style='height:100px;width:200px'><div id=c style='display:grid;width:50%;height:50%;min-width:80px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "`min-width: 0` states a bound",
            "<div style='display:flex;width:200px'><div id=c style='width:50%;height:100px;aspect-ratio:1;min-width:0'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "`min-height: min-content` carries nothing across",
            "<div style='display:flex;width:200px'><div id=c style='display:grid;width:50%;height:100px;aspect-ratio:1;min-height:min-content'></div></div>",
            &[("c", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a `calc()` with a percentage and a `max-width`",
            "<div style='display:flex;width:200px'><div id=c style='display:grid;width:calc(50% + 10px);height:100px;aspect-ratio:1;max-width:60px'></div></div>",
            &[("c", 8.0, 8.0, 60.0, 100.0)],
        ),
        (
            "a `calc()` with a percentage and no bound",
            "<div style='display:flex;width:200px'><div id=c style='display:grid;width:calc(50% + 10px);height:100px;aspect-ratio:1'></div></div>",
            &[("c", 8.0, 8.0, 110.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}

/// A percentage size against a definite basis resolves, and with both sizes given the ratio changes nothing.
#[test]
fn a_percentage_that_resolves_leaves_the_ratio_nothing_to_do() {
    let cases: &[Case<'_>] = &[
        (
            "percentage sizes in a box with a height",
            "<div id=p style='width:200px;height:100px'><div id=a style='width:50%;height:50%;aspect-ratio:3/1'></div></div>",
            &[("p", 8.0, 8.0, 200.0, 100.0), ("a", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "a percentage width in a fixed column",
            "<div id=g style='display:grid;grid-template-columns:200px'><div id=a style='width:50%;height:100px;aspect-ratio:1/1'></div></div>",
            &[("g", 8.0, 8.0, 784.0, 100.0), ("a", 8.0, 8.0, 100.0, 100.0)],
        ),
    ];
    check_boxes(cases);
}
