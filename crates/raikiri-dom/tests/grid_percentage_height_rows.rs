//! A grid item whose `height`, `min-height` or `max-height` is a percentage of
//! its grid area contributes to the rows once at the height an unknown area
//! gives it, and again once the columns are sized against those rows. An item
//! with an `aspect-ratio` widens its column on the second pass: `aspect-ratio:
//! 2/1; height: 50%` with 25px borders in a 75px row of a 100px grid takes the
//! 125px the height of that row gives, and lays out 137.5px wide in the
//! 87.5px row that follows. An `auto` grid keeps the height of the first pass
//! and lets the rows overflow it, as Chromium does.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport, with the body margin of 8px.

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

/// A case: its name, its markup, and the border boxes by id.
type Case = (&'static str, &'static str, &'static [(&'static str, Rect)]);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
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
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(name: &str, body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(box_id, _)| box_id == id)
            .unwrap_or_else(|| panic!("{name}: no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{name}: {body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

#[rustfmt::skip]
const CASES: &[Case] = &[
    (
        "auto-cb25-s75",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 150.0)), ("i", (8.0, 8.0, 137.5, 93.75)), ("c", (8.0, 95.5, 125.0, 75.0)), ("after", (8.0, 158.0, 784.0, 5.0))],
    ),
    (
        "auto-cb25-none",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 75.0)), ("i", (8.0, 8.0, 137.5, 93.75)), ("after", (8.0, 83.0, 784.0, 5.0))],
    ),
    (
        "auto-bb-s75",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 125.0)), ("i", (8.0, 8.0, 50.0, 25.0)), ("c", (8.0, 58.0, 100.0, 75.0)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "auto-bb-none",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 50.0, 25.0)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "auto-cbp-s75",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 175.0)), ("i", (8.0, 8.0, 100.0, 100.0)), ("c", (8.0, 108.0, 100.0, 75.0)), ("after", (8.0, 183.0, 784.0, 5.0))],
    ),
    (
        "auto-cbp-none",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0)), ("after", (8.0, 108.0, 784.0, 5.0))],
    ),
    (
        "auto-calc-s75",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 125.0)), ("i", (8.0, 8.0, 70.0, 35.0)), ("c", (8.0, 58.0, 100.0, 75.0)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "auto-calc-none",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 70.0, 35.0)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "auto-js-s75",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 125.0)), ("i", (8.0, 8.0, 100.0, 75.0)), ("c", (8.0, 58.0, 100.0, 75.0)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "auto-js-none",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 100.0, 75.0)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "auto-txt-s75",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 175.0)), ("i", (8.0, 8.0, 50.0, 50.0)), ("c", (8.0, 108.0, 100.0, 75.0)), ("after", (8.0, 183.0, 784.0, 5.0))],
    ),
    (
        "auto-txt-none",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 50.0, 50.0)), ("after", (8.0, 108.0, 784.0, 5.0))],
    ),
    (
        "h200-cb25-s75",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 162.5, 106.25)), ("c", (8.0, 120.5, 150.0, 75.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-cb25-none",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 250.0, 150.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-bb-s75",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 87.5, 43.75)), ("c", (8.0, 95.5, 100.0, 75.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-bb-none",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 200.0, 100.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-cbp-s75",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 114.0, 114.0)), ("c", (8.0, 125.5, 110.0, 75.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-cbp-none",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 180.0, 180.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-calc-s75",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 109.375, 54.6875)), ("c", (8.0, 97.375, 107.5, 75.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-calc-none",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 220.0, 110.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-js-s75",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 137.5, 93.75)), ("c", (8.0, 95.5, 137.5, 75.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-js-none",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 250.0, 150.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-txt-s75",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 56.25, 56.25)), ("c", (8.0, 120.5, 100.0, 75.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "h200-txt-none",
        "<div id=g style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 200.0)), ("i", (8.0, 8.0, 100.0, 100.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "mc-cb25-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 125.0)), ("i", (8.0, 8.0, 125.0, 87.5)), ("c", (8.0, 83.0, 100.0, 75.0)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "mc-cb25-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 125.0, 87.5)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "mc-bb-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 75.0)), ("i", (8.0, 8.0, 0.0, 0.0)), ("c", (8.0, 8.0, 0.0, 75.0)), ("after", (8.0, 83.0, 784.0, 5.0))],
    ),
    (
        "mc-bb-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 0.0)), ("i", (8.0, 8.0, 0.0, 0.0)), ("after", (8.0, 8.0, 784.0, 5.0))],
    ),
    (
        "mc-cbp-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 95.0)), ("i", (8.0, 8.0, 48.7969, 48.7969)), ("c", (8.0, 44.0, 36.0, 75.0)), ("after", (8.0, 103.0, 784.0, 5.0))],
    ),
    (
        "mc-cbp-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 20.0)), ("i", (8.0, 8.0, 48.7969, 48.7969)), ("after", (8.0, 28.0, 784.0, 5.0))],
    ),
    (
        "mc-calc-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 75.0)), ("i", (8.0, 8.0, 30.0, 15.0)), ("c", (8.0, 18.0, 20.0, 75.0)), ("after", (8.0, 83.0, 784.0, 5.0))],
    ),
    (
        "mc-calc-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 0.0)), ("i", (8.0, 8.0, 30.0, 15.0)), ("after", (8.0, 8.0, 784.0, 5.0))],
    ),
    (
        "mc-js-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 125.0)), ("i", (8.0, 8.0, 100.0, 75.0)), ("c", (8.0, 58.0, 100.0, 75.0)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "mc-js-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 100.0, 75.0)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "mc-txt-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 95.0)), ("i", (8.0, 8.0, 20.0, 10.0)), ("c", (8.0, 28.0, 20.0, 75.0)), ("after", (8.0, 103.0, 784.0, 5.0))],
    ),
    (
        "mc-txt-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:min-content'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 20.0)), ("i", (8.0, 8.0, 20.0, 10.0)), ("after", (8.0, 28.0, 784.0, 5.0))],
    ),
    (
        "two-cb25-s75",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 87.5)), ("i", (8.0, 8.0, 159.375, 104.6875)), ("c", (176.75, 8.0, 31.25, 75.0)), ("after", (8.0, 95.5, 784.0, 5.0))],
    ),
    (
        "two-cb25-none",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 87.5)), ("i", (8.0, 8.0, 159.375, 104.6875)), ("after", (8.0, 95.5, 784.0, 5.0))],
    ),
    (
        "two-bb-s75",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 75.0)), ("i", (8.0, 8.0, 75.0, 37.5)), ("c", (145.5, 8.0, 62.5, 75.0)), ("after", (8.0, 83.0, 784.0, 5.0))],
    ),
    (
        "two-bb-none",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 50.0)), ("i", (8.0, 8.0, 62.5, 31.25)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "two-cbp-s75",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 110.0)), ("i", (8.0, 8.0, 143.1875, 143.1875)), ("c", (162.0, 8.0, 46.0, 75.0)), ("after", (8.0, 118.0, 784.0, 5.0))],
    ),
    (
        "two-cbp-none",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 110.0)), ("i", (8.0, 8.0, 143.1875, 143.1875)), ("after", (8.0, 118.0, 784.0, 5.0))],
    ),
    (
        "two-calc-s75",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 75.0)), ("i", (8.0, 8.0, 95.0, 47.5)), ("c", (155.5, 8.0, 52.5, 75.0)), ("after", (8.0, 83.0, 784.0, 5.0))],
    ),
    (
        "two-calc-none",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 50.0)), ("i", (8.0, 8.0, 87.5, 43.75)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "two-js-s75",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 75.0)), ("i", (8.0, 8.0, 125.0, 87.5)), ("c", (170.5, 8.0, 37.5, 75.0)), ("after", (8.0, 83.0, 784.0, 5.0))],
    ),
    (
        "two-js-none",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 50.0)), ("i", (8.0, 8.0, 100.0, 75.0)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "two-txt-s75",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 125.0)), ("i", (8.0, 8.0, 65.625, 65.625)), ("c", (139.25, 8.0, 68.75, 75.0)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "two-txt-none",
        "<div id=g style='display:grid;width:200px;grid-template-columns:auto auto'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 125.0)), ("i", (8.0, 8.0, 65.625, 65.625)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "rows-cb25-s75",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 155.0)), ("i", (8.0, 8.0, 130.0, 90.0)), ("c", (8.0, 88.0, 130.0, 75.0)), ("after", (8.0, 163.0, 784.0, 5.0))],
    ),
    (
        "rows-cb25-none",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 80.0)), ("i", (8.0, 8.0, 130.0, 90.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "rows-bb-s75",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 155.0)), ("i", (8.0, 8.0, 80.0, 40.0)), ("c", (8.0, 88.0, 100.0, 75.0)), ("after", (8.0, 163.0, 784.0, 5.0))],
    ),
    (
        "rows-bb-none",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 80.0)), ("i", (8.0, 8.0, 80.0, 40.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "rows-cbp-s75",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 155.0)), ("i", (8.0, 8.0, 84.0, 84.0)), ("c", (8.0, 88.0, 100.0, 75.0)), ("after", (8.0, 163.0, 784.0, 5.0))],
    ),
    (
        "rows-cbp-none",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 80.0)), ("i", (8.0, 8.0, 84.0, 84.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "rows-calc-s75",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 155.0)), ("i", (8.0, 8.0, 100.0, 50.0)), ("c", (8.0, 88.0, 100.0, 75.0)), ("after", (8.0, 163.0, 784.0, 5.0))],
    ),
    (
        "rows-calc-none",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 80.0)), ("i", (8.0, 8.0, 100.0, 50.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "rows-js-s75",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 155.0)), ("i", (8.0, 8.0, 130.0, 90.0)), ("c", (8.0, 88.0, 130.0, 75.0)), ("after", (8.0, 163.0, 784.0, 5.0))],
    ),
    (
        "rows-js-none",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 80.0)), ("i", (8.0, 8.0, 130.0, 90.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "rows-txt-s75",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 155.0)), ("i", (8.0, 8.0, 40.0, 40.0)), ("c", (8.0, 88.0, 100.0, 75.0)), ("after", (8.0, 163.0, 784.0, 5.0))],
    ),
    (
        "rows-txt-none",
        "<div id=g style='display:grid;width:100px;grid-template-rows:80px'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 80.0)), ("i", (8.0, 8.0, 40.0, 40.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "fr-cb25-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 150.0)), ("i", (8.0, 8.0, 137.5, 93.75)), ("c", (8.0, 95.5, 125.0, 75.0)), ("after", (8.0, 158.0, 784.0, 5.0))],
    ),
    (
        "fr-cb25-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 75.0)), ("i", (8.0, 8.0, 137.5, 93.75)), ("after", (8.0, 83.0, 784.0, 5.0))],
    ),
    (
        "fr-bb-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 125.0)), ("i", (8.0, 8.0, 50.0, 25.0)), ("c", (8.0, 58.0, 100.0, 75.0)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "fr-bb-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:2/1;height:50%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 50.0, 25.0)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "fr-cbp-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 175.0)), ("i", (8.0, 8.0, 100.0, 100.0)), ("c", (8.0, 108.0, 100.0, 75.0)), ("after", (8.0, 183.0, 784.0, 5.0))],
    ),
    (
        "fr-cbp-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:1/1;box-sizing:content-box;padding:10px;height:80%'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 100.0, 100.0)), ("after", (8.0, 108.0, 784.0, 5.0))],
    ),
    (
        "fr-calc-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 125.0)), ("i", (8.0, 8.0, 70.0, 35.0)), ("c", (8.0, 58.0, 100.0, 75.0)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "fr-calc-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:2/1;height:calc(50% + 10px)'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 70.0, 35.0)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "fr-js-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 125.0)), ("i", (8.0, 8.0, 100.0, 75.0)), ("c", (8.0, 58.0, 100.0, 75.0)), ("after", (8.0, 133.0, 784.0, 5.0))],
    ),
    (
        "fr-js-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%;justify-self:start'><div style='height:10px'></div></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 100.0, 75.0)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
    (
        "fr-txt-s75",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 175.0)), ("i", (8.0, 8.0, 50.0, 50.0)), ("c", (8.0, 108.0, 100.0, 75.0)), ("after", (8.0, 183.0, 784.0, 5.0))],
    ),
    (
        "fr-txt-none",
        "<div id=g style='display:grid;width:100px;grid-template-columns:1fr'><div id=i style='aspect-ratio:1/1;height:50%'>XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 50.0, 50.0)), ("after", (8.0, 108.0, 784.0, 5.0))],
    ),
    (
        "minh-r1-txt",
        "<div id=g style='display:grid;width:200px;grid-template-columns:1fr 1fr;min-height:120px'><div id=i style='aspect-ratio:1/1;height:100%'>XX XX</div><div id=c>XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 120.0)), ("i", (8.0, 8.0, 120.0, 120.0)), ("c", (128.0, 8.0, 80.0, 120.0)), ("after", (8.0, 128.0, 784.0, 5.0))],
    ),
    (
        "minh-r1w-txt",
        "<div id=g style='display:grid;width:200px;grid-template-columns:1fr 1fr;min-height:120px'><div id=i style='aspect-ratio:1/1;height:100%;justify-self:start'>XX XX</div><div id=c>XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 120.0)), ("i", (8.0, 8.0, 120.0, 120.0)), ("c", (128.0, 8.0, 80.0, 120.0)), ("after", (8.0, 128.0, 784.0, 5.0))],
    ),
    (
        "cols-h100-box",
        "<div id=g style='display:grid;width:200px;grid-template-columns:1fr 1fr'><div id=i style='height:100%;background:red'>XX XX</div><div id=c style='height:60px'></div><div id=d style='height:20px'></div><div id=e>X</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 80.0)), ("i", (8.0, 8.0, 100.0, 60.0)), ("c", (108.0, 8.0, 100.0, 60.0)), ("d", (8.0, 68.0, 100.0, 20.0)), ("e", (108.0, 68.0, 100.0, 20.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "pad-h100-txt",
        "<div id=g style='display:grid;width:200px;grid-template-columns:1fr 1fr;padding:7px;border:3px solid'><div id=i style='height:100%;background:red'>XX XX</div><div id=c>XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 100.0)), ("i", (18.0, 18.0, 90.0, 80.0)), ("c", (108.0, 18.0, 90.0, 80.0)), ("after", (8.0, 108.0, 784.0, 5.0))],
    ),
    (
        "maxh-mh100-box",
        "<div id=g style='display:grid;width:200px;grid-template-columns:1fr 1fr;max-height:30px'><div id=i style='min-height:100%'>XX XX</div><div id=c style='height:60px'></div><div id=d style='height:20px'></div><div id=e>X</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 30.0)), ("i", (8.0, 8.0, 100.0, 60.0)), ("c", (108.0, 8.0, 100.0, 60.0)), ("d", (8.0, 68.0, 100.0, 20.0)), ("e", (108.0, 68.0, 100.0, 20.0)), ("after", (8.0, 38.0, 784.0, 5.0))],
    ),
    (
        "prow-h50t-box",
        "<div id=g style='display:grid;width:200px;grid-template-columns:1fr 1fr;grid-template-rows:50% auto'><div id=i style='height:50%'>XX XX</div><div id=c style='height:60px'></div><div id=d style='height:20px'></div><div id=e>X</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 80.0)), ("i", (8.0, 8.0, 100.0, 20.0)), ("c", (108.0, 8.0, 100.0, 60.0)), ("d", (8.0, 48.0, 100.0, 20.0)), ("e", (108.0, 48.0, 100.0, 40.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "frrow-calc-box",
        "<div id=g style='display:grid;width:200px;grid-template-columns:1fr 1fr;grid-template-rows:1fr 2fr'><div id=i style='height:calc(100% - 10px)'>XX XX</div><div id=c style='height:60px'></div><div id=d style='height:20px'></div><div id=e>X</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 180.0)), ("i", (8.0, 8.0, 100.0, 50.0)), ("c", (108.0, 8.0, 100.0, 60.0)), ("d", (8.0, 68.0, 100.0, 20.0)), ("e", (108.0, 68.0, 100.0, 120.0)), ("after", (8.0, 188.0, 784.0, 5.0))],
    ),
    (
        "gap-xh50-txt",
        "<div id=g style='display:grid;width:200px;grid-template-columns:1fr 1fr;gap:10%'><div id=i style='max-height:50%'>XX XX</div><div id=c>XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX XX</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 80.0)), ("i", (8.0, 8.0, 90.0, 40.0)), ("c", (118.0, 8.0, 90.0, 80.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "ac-h100p-box",
        "<div id=g style='display:grid;width:200px;grid-template-columns:1fr 1fr;align-content:center'><div id=i style='height:100%;padding:5px'>XX XX</div><div id=c style='height:60px'></div><div id=d style='height:20px'></div><div id=e>X</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 80.0)), ("i", (8.0, 8.0, 100.0, 60.0)), ("c", (108.0, 8.0, 100.0, 60.0)), ("d", (8.0, 68.0, 100.0, 20.0)), ("e", (108.0, 68.0, 100.0, 20.0)), ("after", (8.0, 88.0, 784.0, 5.0))],
    ),
    (
        "auto-r1-box",
        "<div id=g style='display:grid;width:200px'><div id=i style='aspect-ratio:1/1;height:100%'>XX XX</div><div id=c style='height:60px'></div><div id=d style='height:20px'></div><div id=e>X</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 290.0)), ("i", (8.0, 8.0, 200.0, 200.0)), ("c", (8.0, 208.0, 200.0, 60.0)), ("d", (8.0, 268.0, 200.0, 20.0)), ("e", (8.0, 288.0, 200.0, 10.0)), ("after", (8.0, 298.0, 784.0, 5.0))],
    ),
    (
        "nest",
        "<div id=g style='display:grid;width:200px'><div id=h style='display:grid;height:100%'><div id=i style='height:50%'>X</div><div id=j style='height:40px'></div></div><div id=c style='height:60px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 110.0)), ("h", (8.0, 8.0, 200.0, 50.0)), ("i", (8.0, 8.0, 200.0, 5.0)), ("j", (8.0, 18.0, 200.0, 40.0)), ("c", (8.0, 58.0, 200.0, 60.0)), ("after", (8.0, 118.0, 784.0, 5.0))],
    ),
    (
        "flexin",
        "<div id=f style='display:flex'><div id=g style='display:grid;width:200px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'></div><div id=c style='height:75px'></div></div></div><div id=after style='height:5px'></div>",
        &[("f", (8.0, 8.0, 784.0, 200.0)), ("g", (8.0, 8.0, 200.0, 200.0)), ("i", (8.0, 8.0, 175.0, 112.5)), ("c", (8.0, 133.0, 200.0, 75.0)), ("after", (8.0, 208.0, 784.0, 5.0))],
    ),
    (
        "abs",
        "<div id=g style='display:grid;width:200px;position:relative'><div id=i style='position:absolute;height:50%;width:10px'></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 200.0, 75.0)), ("i", (8.0, 8.0, 10.0, 37.5)), ("c", (8.0, 8.0, 200.0, 75.0)), ("after", (8.0, 83.0, 784.0, 5.0))],
    ),
    (
        "x-word",
        "<div id=g style='display:grid;width:300px'><div id=i style='aspect-ratio:2/1;height:50%'>WWWWWWWWWWWWWWWWWWWW</div><div id=c style='height:40px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 300.0, 190.0)), ("i", (8.0, 8.0, 200.0, 75.0)), ("c", (8.0, 158.0, 300.0, 40.0)), ("after", (8.0, 198.0, 784.0, 5.0))],
    ),
    (
        "x-word2",
        "<div id=g style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;height:50%'>WWWWWWWWWWWW</div><div id=c style='height:40px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 120.0, 30.0)), ("c", (8.0, 68.0, 120.0, 40.0)), ("after", (8.0, 108.0, 784.0, 5.0))],
    ),
    (
        "x-accenter",
        "<div id=g style='display:grid;width:100px;align-content:center'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:50%'><div style='height:10px'></div></div><div id=c style='height:75px'></div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 150.0)), ("i", (8.0, 1.75, 137.5, 93.75)), ("c", (8.0, 89.25, 125.0, 75.0)), ("after", (8.0, 158.0, 784.0, 5.0))],
    ),
    (
        "x-autofill",
        "<div id=g style='display:grid;width:100px;grid-template-rows:repeat(auto-fill,10px)'><div id=i style='aspect-ratio:2/1;height:100%'></div><div id=c style='height:20px'>X</div><div id=d style='height:20px'>X</div></div><div id=after style='height:5px'></div>",
        &[("g", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 20.0, 10.0)), ("c", (8.0, 18.0, 100.0, 20.0)), ("d", (8.0, 38.0, 100.0, 20.0)), ("after", (8.0, 58.0, 784.0, 5.0))],
    ),
];

/// Run every case whose name satisfies `select`, and make sure some do.
fn check_group(select: fn(&str) -> bool) {
    let cases: Vec<&Case> = CASES.iter().filter(|case| select(case.0)).collect();
    assert!(!cases.is_empty(), "no case selected");
    for &&(name, body, expected) in &cases {
        check(name, body, expected);
    }
}

#[test]
fn ratio_items_widen_their_column_once_the_rows_are_sized() {
    check_group(|name| {
        ["auto-", "mc-", "two-", "fr-", "flexin"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
    });
}

#[test]
fn ratio_items_in_definite_rows_take_the_width_of_their_area_height() {
    check_group(|name| name.starts_with("h200-") || name.starts_with("rows-"));
}

#[test]
fn content_alignment_and_repeated_rows_follow_the_first_pass() {
    check_group(|name| name.starts_with("x-"));
}

#[test]
fn items_filling_their_area_leave_the_grid_as_the_first_pass_sized_it() {
    check_group(|name| {
        [
            "minh-", "cols-", "pad-", "maxh-", "prow-", "frrow-", "gap-", "ac-", "nest", "abs",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    });
}
