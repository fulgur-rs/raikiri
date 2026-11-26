//! An item of a flex container on vertical axes that states its main size and
//! has a preferred aspect ratio takes its cross size from the main size it
//! flexes to, as an item of a horizontal container does. In a `vertical-rl`
//! row of 100px, `height: 20px; aspect-ratio: 1/1; flex-grow: 1` grows to
//! 100px high and is 100px wide. A cross size the item states keeps the size
//! it has, whatever limits of the main axis the ratio would carry across.
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
const FOLLOWS_MAIN: &[Case] = &[
    (
        "vertical-rl-row-p100-h",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "vertical-rl-row-p100-h2",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "vertical-rl-row-p100-hb",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1;border:2px solid;padding:3px'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "vertical-rl-row-reverse-p100-h",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "vertical-rl-row-reverse-p100-h2",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "vertical-rl-row-reverse-p100-hb",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1;border:2px solid;padding:3px'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "vertical-lr-row-p100-h",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "vertical-lr-row-p100-h2",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "vertical-lr-row-p100-hb",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1;border:2px solid;padding:3px'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "vertical-lr-row-reverse-p100-h",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "vertical-lr-row-reverse-p100-h2",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "vertical-lr-row-reverse-p100-hb",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1;border:2px solid;padding:3px'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "sideways-rl-row-p100-h",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-rl;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "sideways-rl-row-p100-h2",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-rl;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "sideways-rl-row-p100-hb",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-rl;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1;border:2px solid;padding:3px'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "sideways-rl-row-reverse-p100-h",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-rl;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "sideways-rl-row-reverse-p100-h2",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-rl;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "sideways-rl-row-reverse-p100-hb",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-rl;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1;border:2px solid;padding:3px'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "sideways-lr-row-p100-h",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-lr;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "sideways-lr-row-p100-h2",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-lr;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "sideways-lr-row-p100-hb",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-lr;flex-direction:row;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1;border:2px solid;padding:3px'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "sideways-lr-row-reverse-p100-h",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-lr;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "sideways-lr-row-reverse-p100-h2",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-lr;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "sideways-lr-row-reverse-p100-hb",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-lr;flex-direction:row-reverse;height:100px'><div id=t style='height:20px;aspect-ratio:1/1;flex-grow:1;border:2px solid;padding:3px'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-grow",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-shrink2",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:80px;aspect-ratio:1/1'></div><div id=t1 style='height:80px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 50.0, 100.0)), ("t0", (8.0, 8.0, 50.0, 50.0)), ("t1", (8.0, 58.0, 50.0, 50.0))],
    ),
    (
        "b-vertical-rl-r-basis",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20px;flex-basis:60px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 60.0, 100.0)), ("t0", (8.0, 8.0, 60.0, 60.0))],
    ),
    (
        "b-vertical-rl-r-maxh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20px;max-height:50px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 50.0, 100.0)), ("t0", (8.0, 8.0, 50.0, 50.0))],
    ),
    (
        "b-vertical-rl-r-maxw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20px;max-width:40px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 40.0, 100.0)), ("t0", (8.0, 8.0, 40.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-minw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20px;min-width:150px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 150.0, 100.0)), ("t0", (8.0, 8.0, 150.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-text",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1' >XX XX XXXXXXXX</div><div style=''></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-flex",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='display:flex;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-grid",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='display:grid;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-start",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:start'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-center",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:center'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-htb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='writing-mode:horizontal-tb;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-pct",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20%;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-cb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='box-sizing:content-box;padding:5px;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-two",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:30px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 110.0, 100.0)), ("t0", (8.0, 8.0, 110.0, 45.0)), ("t1", (8.0, 53.0, 110.0, 55.0))],
    ),
    (
        "b-vertical-rl-r-wrap",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;flex-wrap:wrap'><div id=t0 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (108.0, 8.0, 100.0, 100.0)), ("t1", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-minw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;min-width:150px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-start",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:start'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-center",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:center'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-c-minw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:20px;min-width:150px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (-42.0, 8.0, 150.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-minw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;min-width:150px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (-42.0, 8.0, 150.0, 20.0))],
    ),
    (
        "b-vertical-lr-r-grow",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-shrink2",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:80px;aspect-ratio:1/1'></div><div id=t1 style='height:80px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 50.0, 100.0)), ("t0", (8.0, 8.0, 50.0, 50.0)), ("t1", (8.0, 58.0, 50.0, 50.0))],
    ),
    (
        "b-vertical-lr-r-basis",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20px;flex-basis:60px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 60.0, 100.0)), ("t0", (8.0, 8.0, 60.0, 60.0))],
    ),
    (
        "b-vertical-lr-r-maxh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20px;max-height:50px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 50.0, 100.0)), ("t0", (8.0, 8.0, 50.0, 50.0))],
    ),
    (
        "b-vertical-lr-r-maxw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20px;max-width:40px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 40.0, 100.0)), ("t0", (8.0, 8.0, 40.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-minw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20px;min-width:150px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 150.0, 100.0)), ("t0", (8.0, 8.0, 150.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-text",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1' >XX XX XXXXXXXX</div><div style=''></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-flex",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='display:flex;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-grid",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='display:grid;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-start",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:start'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-center",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:center'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-htb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='writing-mode:horizontal-tb;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-pct",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20%;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-cb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='box-sizing:content-box;padding:5px;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-lr-r-two",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:30px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 110.0, 100.0)), ("t0", (8.0, 8.0, 110.0, 45.0)), ("t1", (8.0, 53.0, 110.0, 55.0))],
    ),
    (
        "b-vertical-lr-r-wrap",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px;flex-wrap:wrap'><div id=t0 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 100.0, 100.0)), ("t1", (108.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-lr-rw-minw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;min-width:150px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-lr-rw-start",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:start'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-lr-rw-center",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:center'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-lr-c-minw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:column;width:100px'><div id=t0 style='height:20px;min-width:150px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 150.0, 20.0))],
    ),
    (
        "b-vertical-lr-ch-minw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;min-width:150px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 150.0, 20.0))],
    ),
];

#[rustfmt::skip]
const UNCHANGED: &[Case] = &[
    (
        "vertical-rl-row-p100-w",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "vertical-rl-row-reverse-p100-w",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row-reverse;height:100px'><div id=t style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "vertical-lr-row-p100-w",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row;height:100px'><div id=t style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "vertical-lr-row-reverse-p100-w",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:vertical-lr;flex-direction:row-reverse;height:100px'><div id=t style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "sideways-rl-row-p100-w",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-rl;flex-direction:row;height:100px'><div id=t style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "sideways-rl-row-reverse-p100-w",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-rl;flex-direction:row-reverse;height:100px'><div id=t style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "sideways-lr-row-p100-w",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-lr;flex-direction:row;height:100px'><div id=t style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "sideways-lr-row-reverse-p100-w",
        "<div style='width:100px'><div id=p style='display:flex;writing-mode:sideways-lr;flex-direction:row-reverse;height:100px'><div id=t style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-basisnoh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='flex-basis:60px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 60.0, 100.0)), ("t0", (8.0, 8.0, 60.0, 60.0))],
    ),
    (
        "b-vertical-rl-r-htbw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='writing-mode:horizontal-tb;width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t0", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-w",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t0", (8.0, 8.0, 20.0, 100.0))],
    ),
    (
        "b-vertical-rl-r-wmaxh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px'><div id=t0 style='width:20px;max-height:50px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 20.0, 100.0)), ("t0", (8.0, 8.0, 20.0, 50.0))],
    ),
    (
        "b-vertical-rl-rw-grow",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-shrink2",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:80px;aspect-ratio:1/1'></div><div id=t1 style='height:80px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 80.0)), ("t1", (8.0, 88.0, 200.0, 80.0))],
    ),
    (
        "b-vertical-rl-rw-basis",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;flex-basis:60px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 60.0))],
    ),
    (
        "b-vertical-rl-rw-basisnoh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='flex-basis:60px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 200.0))],
    ),
    (
        "b-vertical-rl-rw-maxh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;max-height:50px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 50.0))],
    ),
    (
        "b-vertical-rl-rw-maxw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;max-width:40px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (168.0, 8.0, 40.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-text",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1' >XX XX XXXXXXXX</div><div style=''></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-flex",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='display:flex;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-grid",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='display:grid;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-htb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='writing-mode:horizontal-tb;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-htbw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='writing-mode:horizontal-tb;width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (188.0, 8.0, 20.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-pct",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20%;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-w",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (188.0, 8.0, 20.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-wmaxh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='width:20px;max-height:50px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (188.0, 8.0, 20.0, 50.0))],
    ),
    (
        "b-vertical-rl-rw-cb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='box-sizing:content-box;padding:5px;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 100.0))],
    ),
    (
        "b-vertical-rl-rw-two",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:30px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (8.0, 8.0, 200.0, 45.0)), ("t1", (8.0, 53.0, 200.0, 55.0))],
    ),
    (
        "b-vertical-rl-rw-wrap",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:row;height:100px;width:200px;flex-wrap:wrap'><div id=t0 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 200.0, 100.0)), ("t0", (108.0, 8.0, 100.0, 100.0)), ("t1", (8.0, 8.0, 100.0, 100.0))],
    ),
    (
        "b-vertical-rl-c-grow",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-shrink2",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:80px;aspect-ratio:1/1'></div><div id=t1 style='height:80px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 80.0)), ("t0", (28.0, 8.0, 80.0, 80.0)), ("t1", (-52.0, 8.0, 80.0, 80.0))],
    ),
    (
        "b-vertical-rl-c-basis",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:20px;flex-basis:60px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (48.0, 8.0, 60.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-maxh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:20px;max-height:50px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-maxw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:20px;max-width:40px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (68.0, 8.0, 40.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-text",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1' >XX XX XXXXXXXX</div><div style=''></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-flex",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='display:flex;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-grid",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='display:grid;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-start",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:start'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-center",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:center'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-htb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='writing-mode:horizontal-tb;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-htbw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='writing-mode:horizontal-tb;width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-w",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-wmaxh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='width:20px;max-height:50px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 20.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-c-cb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='box-sizing:content-box;padding:5px;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 30.0)), ("t0", (8.0, 8.0, 100.0, 30.0))],
    ),
    (
        "b-vertical-rl-c-two",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:30px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 30.0)), ("t0", (78.0, 8.0, 30.0, 20.0)), ("t1", (8.0, 8.0, 70.0, 30.0))],
    ),
    (
        "b-vertical-rl-c-wrap",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;flex-wrap:wrap'><div id=t0 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 120.0)), ("t0", (8.0, 8.0, 100.0, 60.0)), ("t1", (8.0, 68.0, 100.0, 60.0))],
    ),
    (
        "b-vertical-rl-ch-grow",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-shrink2",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:80px;aspect-ratio:1/1'></div><div id=t1 style='height:80px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (28.0, 8.0, 80.0, 80.0)), ("t1", (-52.0, 8.0, 80.0, 80.0))],
    ),
    (
        "b-vertical-rl-ch-basis",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;flex-basis:60px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (48.0, 8.0, 60.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-basisnoh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='flex-basis:60px;aspect-ratio:1/1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (-92.0, 8.0, 200.0, 200.0))],
    ),
    (
        "b-vertical-rl-ch-maxh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;max-height:50px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-maxw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;max-width:40px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (68.0, 8.0, 40.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-text",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1' >XX XX XXXXXXXX</div><div style=''></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-flex",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='display:flex;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-grid",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='display:grid;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-start",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:start'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-center",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;aspect-ratio:2/1;flex-grow:1;align-self:center'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 98.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-htb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='writing-mode:horizontal-tb;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 20.0))],
    ),
    (
        "b-vertical-rl-ch-htbw",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='writing-mode:horizontal-tb;width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 200.0))],
    ),
    (
        "b-vertical-rl-ch-pct",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20%;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 40.0))],
    ),
    (
        "b-vertical-rl-ch-w",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='width:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 200.0))],
    ),
    (
        "b-vertical-rl-ch-wmaxh",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='width:20px;max-height:50px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 50.0))],
    ),
    (
        "b-vertical-rl-ch-cb",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='box-sizing:content-box;padding:5px;height:20px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 30.0))],
    ),
    (
        "b-vertical-rl-ch-two",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px'><div id=t0 style='height:20px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:30px;aspect-ratio:2/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (78.0, 8.0, 30.0, 20.0)), ("t1", (8.0, 8.0, 70.0, 30.0))],
    ),
    (
        "b-vertical-rl-ch-wrap",
        "<div style='width:300px'><div id=p style='display:flex;writing-mode:vertical-rl;flex-direction:column;width:100px;height:200px;flex-wrap:wrap'><div id=t0 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div><div id=t1 style='height:60px;aspect-ratio:1/1;flex-grow:1'></div></div></div>",
        &[("p", (8.0, 8.0, 100.0, 200.0)), ("t0", (8.0, 8.0, 100.0, 60.0)), ("t1", (8.0, 108.0, 100.0, 60.0))],
    ),
];

#[test]
fn ratio_items_of_a_vertical_row_take_the_cross_size_of_their_flexed_main_size() {
    for &(name, body, expected) in FOLLOWS_MAIN {
        check(name, body, expected);
    }
}

#[test]
fn other_ratio_items_of_a_vertical_container_keep_their_size() {
    for &(name, body, expected) in UNCHANGED {
        check(name, body, expected);
    }
}
