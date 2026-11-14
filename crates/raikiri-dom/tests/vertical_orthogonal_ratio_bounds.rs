//! A block-level box in a vertical writing mode under a horizontal block
//! container is an orthogonal flow (CSS Writing Modes 4 §7.3). Its height, the
//! inline size, comes from its content within the room it is offered, and with
//! an `aspect-ratio` and an `auto` width it is held to the bounds the width
//! carries over through the ratio before the ratio gives the width from it
//! (CSS Sizing 4 §5.1): `max-width: 50px` keeps a square box 50px tall however
//! tall its lines are. A height that size containment gives the box stands for
//! its content and is held the same way, and a height the box states keeps
//! its own size.
//!
//! The intrinsic keywords on the block axis behave as `auto` (CSS Sizing 3
//! §3.1), so a vertical box with `width: max-content` or `min-content` takes
//! its width from the ratio as well, whether it is block-level, an
//! inline-block, a float or absolutely positioned.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport with no body margin. Some cases matched before
//! and guard the parts that already worked.

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

fn check_case(&(name, body, expected): &Case) {
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

#[test]
fn an_orthogonal_ratio_box_holds_its_height_to_the_bounds_its_width_carries() {
    BLOCK_LEVEL.iter().for_each(check_case);
}

#[test]
fn an_intrinsic_keyword_width_of_a_vertical_ratio_box_behaves_as_auto() {
    KEYWORD_WIDTH.iter().for_each(check_case);
}

#[test]
fn other_vertical_and_horizontal_ratio_boxes_keep_their_sizes() {
    GUARDS.iter().for_each(check_case);
}

/// Block-level vertical boxes in a horizontal block container.
const BLOCK_LEVEL: &[Case] = &[
    (
        "a01",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a02",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;contain:size;contain-intrinsic-size:200px 200px"></div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a03",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;contain:size;contain-intrinsic-size:200px 200px;padding:5px"></div></div>"#,
        &[("b", (0.0, 0.0, 60.0, 60.0))],
    ),
    (
        "a04",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-lr">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a05",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;display:flow-root">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a06",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;display:flex">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a07",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;display:grid">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a08",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;display:list-item">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a09",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:2/1;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 25.0))],
    ),
    (
        "a10",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;padding:5px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 60.0, 60.0))],
    ),
    (
        "a11",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;padding:5px;box-sizing:border-box">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a12",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;overflow:hidden">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a13",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;min-width:300px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 300.0, 300.0))],
    ),
    (
        "a14",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl">XX</div></div>"#,
        &[("b", (0.0, 0.0, 20.0, 20.0))],
    ),
    (
        "a15",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;max-height:30px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 30.0, 30.0))],
    ),
    (
        "a16",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50%;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "a17",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl">XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX XXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "a18",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;height:300px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 300.0))],
    ),
    (
        "a19",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl"><div id=c style="height:100%;width:10px"></div></div></div>"#,
        &[("b", (0.0, 0.0, 10.0, 0.0)), ("c", (0.0, 0.0, 10.0, 0.0))],
    ),
    (
        "a20",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;min-width:60px;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 60.0, 60.0))],
    ),
    (
        "a21",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl"></div></div>"#,
        &[("b", (0.0, 0.0, 0.0, 0.0))],
    ),
    (
        "a22",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "b01",
        r#"<div style="width:500px;height:100px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "b02",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div><div id=c style="height:10px"></div></div>"#,
        &[
            ("b", (0.0, 0.0, 50.0, 50.0)),
            ("c", (0.0, 50.0, 500.0, 10.0)),
        ],
    ),
    (
        "b03",
        r#"<div style="width:500px"><div style="float:left;width:100px;height:100px"></div><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (100.0, 0.0, 50.0, 50.0))],
    ),
    (
        "b04",
        r#"<table><tr><td id=t><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></td></tr></table>"#,
        &[("t", (2.0, 2.0, 52.0, 52.0)), ("b", (3.0, 3.0, 50.0, 50.0))],
    ),
    (
        "b05",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;margin:10px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (10.0, 10.0, 50.0, 50.0))],
    ),
    (
        "b06",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;padding:10%">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 150.0, 150.0))],
    ),
    (
        "b07",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;border:5px solid">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 60.0, 60.0))],
    ),
    (
        "b08",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1/2;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 100.0))],
    ),
    (
        "b09",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;contain:size"></div></div>"#,
        &[("b", (0.0, 0.0, 0.0, 0.0))],
    ),
    (
        "b10",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;min-height:100px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 100.0))],
    ),
    (
        "b11",
        r#"<div style="writing-mode:vertical-rl;height:500px"><div id=b style="aspect-ratio:1;max-height:50px;writing-mode:horizontal-tb">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "b12",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:auto 1;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "b13",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "b14",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl"><div id=c style="width:100px;height:20px"></div></div></div>"#,
        &[
            ("b", (0.0, 0.0, 50.0, 20.0)),
            ("c", (-50.0, 0.0, 100.0, 20.0)),
        ],
    ),
    (
        "b15",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;display:flex"><div id=c style="height:300px;width:10px"></div></div></div>"#,
        &[
            ("b", (0.0, 0.0, 50.0, 50.0)),
            ("c", (40.0, 0.0, 10.0, 50.0)),
        ],
    ),
    (
        "b16",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;height:50%">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 300.0))],
    ),
    (
        "b17",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;box-sizing:border-box;padding:0 5px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "b18",
        r#"<ul style="width:500px;margin:0"><li id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl">XXXXXXXXXXXXXXXXXXXX</li></ul>"#,
        &[("b", (40.0, 0.0, 50.0, 50.0))],
    ),
];

/// Vertical boxes whose width is an intrinsic keyword, and heights that
/// behave as `auto` or not.
const KEYWORD_WIDTH: &[Case] = &[
    (
        "k01",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "k02",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;width:min-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "k03",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;width:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "k04",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "k05",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;width:max-content;display:inline-block">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "k06",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;width:max-content;float:left">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "k07",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;width:max-content;position:absolute">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "k08",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;height:auto">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "k09",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;height:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "k10",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;height:fit-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "k11",
        r#"<div style="width:500px;height:200px"><div id=b style="aspect-ratio:1;max-width:50px;writing-mode:vertical-rl;height:50%">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 100.0))],
    ),
    (
        "k12",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;height:50%">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 300.0, 300.0))],
    ),
];

/// Boxes that took other paths and already matched.
const GUARDS: &[Case] = &[
    (
        "h01",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-height:50px;width:max-content;display:inline-block">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "h02",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;width:min-content;float:left">XX XX</div></div>"#,
        &[("b", (0.0, 0.0, 20.0, 20.0))],
    ),
    (
        "h03",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;max-height:50px">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "h04",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:2;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 100.0))],
    ),
    (
        "h05",
        r#"<div style="width:500px"><div id=b style="writing-mode:vertical-rl;width:max-content;display:inline-block">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 10.0, 200.0))],
    ),
    (
        "h06",
        r#"<div style="width:500px"><div id=b style="writing-mode:vertical-rl;width:max-content;float:left">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 10.0, 200.0))],
    ),
    (
        "h07",
        r#"<div style="width:500px"><div id=b style="writing-mode:vertical-rl;width:max-content">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 10.0, 200.0))],
    ),
    (
        "h08",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content;display:inline-flex">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "h09",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;width:min-content;float:left;display:grid">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 200.0, 200.0))],
    ),
    (
        "h10",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px;display:inline-block">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
    (
        "h11",
        r#"<div style="width:500px"><div id=b style="aspect-ratio:1;writing-mode:vertical-rl;max-width:50px;position:absolute">XXXXXXXXXXXXXXXXXXXX</div></div>"#,
        &[("b", (0.0, 0.0, 50.0, 50.0))],
    ),
];
