//! A grid container in a vertical writing mode takes its first baseline from
//! an item in its first row (CSS Grid 1 §11.5), also when that item runs
//! against the block direction of the container and aligns by its baseline:
//! the baseline is then the one the container aligned the item by, from its
//! own first line or synthesized from its border box, where the container
//! aligned it, and not where a relative offset moved it.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport with no body margin.

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

/// The border boxes of the elements of `body` with an `id`: where the first
/// fragment of each lies and how wide it is, and the heights of its fragments
/// on all pages summed.
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
    let mut boxes: Vec<(String, Rect)> = Vec::new();
    for page in result.pages() {
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            let rect = fragment.rect();
            match boxes.iter_mut().find(|(box_id, _)| box_id == id) {
                Some((_, found)) => found.3 += rect.height,
                None => boxes.push((id.to_owned(), (rect.x, rect.y, rect.width, rect.height))),
            }
        }
    }
    boxes
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(name: &str, body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(box_id, _)| box_id == id)
            .unwrap_or_else(|| panic!("{name}: no fragment for #{id}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{name}: #{id} is {found:?}, Chromium has {:?}",
            (x, y, width, height)
        );
    }
}

/// The vertical grid container `g` among the baseline-aligned items `a` and
/// `z` of a vertical flex container.
const CASES: &[Case] = &[
    (
        "b1",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (265.0, 10.0, 30.0, 10.0)),
            ("i", (265.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "b2",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;width:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (270.0, 10.0, 30.0, 10.0)),
            ("i", (270.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "b3",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">X</div><div id=j style="writing-mode:vertical-lr;line-height:20px;align-self:baseline">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (245.0, 10.0, 50.0, 10.0)),
            ("i", (265.0, 10.0, 30.0, 10.0)),
            ("j", (245.0, 10.0, 20.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "b4",
        r#"<div style="writing-mode:vertical-lr;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-rl;line-height:40px;align-self:baseline;width:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (10.0, 0.0, 10.0, 10.0)),
            ("g", (5.0, 10.0, 30.0, 10.0)),
            ("i", (5.0, 10.0, 30.0, 10.0)),
            ("z", (0.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "b5",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid;grid-template-columns:auto auto"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">X</div><div id=j style="line-height:20px;align-self:baseline">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (265.0, 10.0, 30.0, 20.0)),
            ("i", (265.0, 10.0, 30.0, 10.0)),
            ("j", (275.0, 20.0, 20.0, 10.0)),
            ("z", (270.0, 30.0, 30.0, 10.0)),
        ],
    ),
    (
        "b6",
        r#"<div style="display:flex;align-items:baseline"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;height:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (0.0, 22.0, 10.0, 10.0)),
            ("g", (10.0, 0.0, 40.0, 30.0)),
            ("i", (10.0, 0.0, 40.0, 30.0)),
            ("z", (50.0, 12.0, 10.0, 30.0)),
        ],
    ),
    (
        "c_pad",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid;padding:3px 7px 0 0;border-right:2px solid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (276.0, 0.0, 10.0, 10.0)),
            ("g", (261.0, 10.0, 39.0, 13.0)),
            ("i", (261.0, 13.0, 30.0, 10.0)),
            ("z", (266.0, 23.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_padl",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid;padding-left:9px;border-left:4px solid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (252.0, 10.0, 43.0, 10.0)),
            ("i", (265.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_rel",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;position:relative;left:6px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (265.0, 10.0, 30.0, 10.0)),
            ("i", (271.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_relr",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;position:relative;right:6px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (265.0, 10.0, 30.0, 10.0)),
            ("i", (259.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_relboth",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;position:relative;left:6px;right:3px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (265.0, 10.0, 30.0, 10.0)),
            ("i", (262.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_relpct",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid;grid-template-rows:100px;width:100px"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;position:relative;left:10%">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (215.0, 0.0, 10.0, 10.0)),
            ("g", (200.0, 10.0, 100.0, 10.0)),
            ("i", (210.0, 10.0, 30.0, 10.0)),
            ("z", (205.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_lrboth",
        r#"<div style="writing-mode:vertical-lr;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-rl;line-height:40px;align-self:baseline;width:30px;position:relative;left:6px;right:3px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (10.0, 0.0, 10.0, 10.0)),
            ("g", (5.0, 10.0, 30.0, 10.0)),
            ("i", (11.0, 10.0, 30.0, 10.0)),
            ("z", (0.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_mar",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;margin-right:5px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (265.0, 10.0, 35.0, 10.0)),
            ("i", (265.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_auto",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;margin-left:auto">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (270.0, 10.0, 30.0, 10.0)),
            ("i", (270.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_last",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:last baseline;width:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (265.0, 10.0, 30.0, 10.0)),
            ("i", (265.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_empty",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px"></div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (270.0, 10.0, 30.0, 0.0)),
            ("i", (270.0, 10.0, 30.0, 0.0)),
            ("z", (270.0, 10.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_rows",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid;grid-template-columns:auto auto"><div id=j style="line-height:20px">X</div><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (265.0, 10.0, 30.0, 20.0)),
            ("j", (265.0, 10.0, 30.0, 10.0)),
            ("i", (265.0, 20.0, 30.0, 10.0)),
            ("z", (270.0, 30.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_two",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">X</div><div id=j style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">XX</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (235.0, 10.0, 60.0, 20.0)),
            ("i", (265.0, 10.0, 30.0, 20.0)),
            ("j", (235.0, 10.0, 30.0, 20.0)),
            ("z", (270.0, 30.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_lr_outer",
        r#"<div style="writing-mode:vertical-lr;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid;padding-left:5px"><div id=i style="writing-mode:vertical-rl;line-height:40px;align-self:baseline;width:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (10.0, 0.0, 10.0, 10.0)),
            ("g", (0.0, 10.0, 35.0, 10.0)),
            ("i", (5.0, 10.0, 30.0, 10.0)),
            ("z", (0.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_inline",
        r#"<div style="writing-mode:vertical-rl;width:300px;height:300px;line-height:10px">X<div id=g style="display:inline-grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">X</div></div>X</div>"#,
        &[
            ("g", (270.0, 10.0, 30.0, 10.0)),
            ("i", (270.0, 10.0, 30.0, 10.0)),
        ],
    ),
    (
        "c_wide",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid;width:80px"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (235.0, 0.0, 10.0, 10.0)),
            ("g", (220.0, 10.0, 80.0, 10.0)),
            ("i", (220.0, 10.0, 30.0, 10.0)),
            ("z", (225.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "d_contain",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;contain:layout">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (270.0, 10.0, 30.0, 10.0)),
            ("i", (270.0, 10.0, 30.0, 10.0)),
            ("z", (270.0, 20.0, 30.0, 10.0)),
        ],
    ),
    (
        "d_box",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;height:20px"></div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (270.0, 10.0, 30.0, 20.0)),
            ("i", (270.0, 10.0, 30.0, 20.0)),
            ("z", (270.0, 30.0, 30.0, 10.0)),
        ],
    ),
    (
        "d_boxrl",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid"><div id=i style="writing-mode:sideways-lr;align-self:baseline;width:30px;height:20px"></div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (270.0, 10.0, 30.0, 20.0)),
            ("i", (270.0, 10.0, 30.0, 20.0)),
            ("z", (270.0, 30.0, 30.0, 10.0)),
        ],
    ),
    (
        "d_span",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid;grid-template-rows:auto auto;grid-auto-flow:column"><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;grid-row:1 / 3">X</div><div id=j style="line-height:20px;align-self:baseline;grid-row:1">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (265.0, 10.0, 30.0, 20.0)),
            ("i", (265.0, 10.0, 30.0, 10.0)),
            ("j", (275.0, 20.0, 20.0, 10.0)),
            ("z", (270.0, 30.0, 30.0, 10.0)),
        ],
    ),
    (
        "d_span2",
        r#"<div style="writing-mode:vertical-rl;display:flex;align-items:baseline;width:300px;height:300px"><div id=a style="line-height:10px">X</div><div id=g style="display:grid;grid-template-columns:auto auto"><div id=j style="line-height:20px;align-self:baseline;grid-column:1">X</div><div id=i style="writing-mode:vertical-lr;line-height:40px;align-self:baseline;width:30px;grid-column:2;grid-row:1 / 3">X</div><div id=k style="line-height:50px;grid-column:1;grid-row:2">X</div></div><div id=z style="line-height:30px">X</div></div>"#,
        &[
            ("a", (280.0, 0.0, 10.0, 10.0)),
            ("g", (225.0, 10.0, 70.0, 20.0)),
            ("j", (275.0, 10.0, 20.0, 10.0)),
            ("i", (225.0, 20.0, 30.0, 10.0)),
            ("k", (225.0, 10.0, 50.0, 10.0)),
            ("z", (270.0, 30.0, 30.0, 10.0)),
        ],
    ),
];

#[test]
fn a_vertical_grid_container_takes_the_baseline_of_an_opposed_first_item() {
    for &(name, body, expected) in CASES {
        check(name, body, expected);
    }
}
