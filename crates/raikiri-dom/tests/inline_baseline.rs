//! Baseline alignment along the inline axis: grid `justify-self` and
//! `justify-items`, and the cross axis of a column flex container.
//!
//! Each expected border box is the one Chromium computes for the same markup
//! in an 800px by 600px viewport, with the body margin of 8px.

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

#[test]
fn grid_horizontal_parallel_baseline_ltr() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;direction:ltr;justify-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (15.0, 11.0, 30.0, 20.0)),
            ("a2", (15.0, 50.0, 20.0, 20.0)),
            ("a3", (15.0, 89.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_horizontal_parallel_last_ltr() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;direction:ltr;justify-items:last baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 11.0, 30.0, 20.0)),
            ("a2", (76.0, 50.0, 20.0, 20.0)),
            ("a3", (76.0, 89.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_horizontal_parallel_baseline_rtl() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;direction:rtl;justify-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 11.0, 30.0, 20.0)),
            ("a2", (76.0, 50.0, 20.0, 20.0)),
            ("a3", (76.0, 89.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_horizontal_parallel_last_rtl() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;direction:rtl;justify-items:last baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (15.0, 11.0, 30.0, 20.0)),
            ("a2", (15.0, 50.0, 20.0, 20.0)),
            ("a3", (15.0, 89.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_parallel_mixed_baseline() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-lr;text-orientation:mixed;justify-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (12.0, 11.0, 30.0, 20.0)),
            ("a2", (55.0, 11.0, 20.0, 20.0)),
            ("a3", (89.0, 11.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_parallel_mixed_last() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-lr;text-orientation:mixed;justify-items:last baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (12.0, 83.0, 30.0, 20.0)),
            ("a2", (55.0, 83.0, 20.0, 20.0)),
            ("a3", (89.0, 83.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_parallel_sideways_baseline() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-lr;text-orientation:sideways;justify-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (12.0, 11.0, 30.0, 20.0)),
            ("a2", (55.0, 11.0, 20.0, 20.0)),
            ("a3", (89.0, 11.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_parallel_sideways_last() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-lr;text-orientation:sideways;justify-items:last baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (12.0, 83.0, 30.0, 20.0)),
            ("a2", (55.0, 83.0, 20.0, 20.0)),
            ("a3", (89.0, 83.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_parallel_mixed_baseline() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-rl;text-orientation:mixed;justify-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 11.0, 30.0, 20.0)),
            ("a2", (47.0, 11.0, 20.0, 20.0)),
            ("a3", (0.0, 11.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_parallel_mixed_last() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-rl;text-orientation:mixed;justify-items:last baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 83.0, 30.0, 20.0)),
            ("a2", (47.0, 83.0, 20.0, 20.0)),
            ("a3", (0.0, 83.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_parallel_sideways_baseline() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-rl;text-orientation:sideways;justify-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 11.0, 30.0, 20.0)),
            ("a2", (47.0, 11.0, 20.0, 20.0)),
            ("a3", (0.0, 11.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_parallel_sideways_last() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-rl;text-orientation:sideways;justify-items:last baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 83.0, 30.0, 20.0)),
            ("a2", (47.0, 83.0, 20.0, 20.0)),
            ("a3", (0.0, 83.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_rtl_parallel_first() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-lr;direction:rtl;justify-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (12.0, 51.0, 30.0, 20.0)),
            ("a2", (55.0, 51.0, 20.0, 20.0)),
            ("a3", (89.0, 51.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_horizontal_items_baseline() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-lr;justify-items:baseline"><div id="a1" style="writing-mode:horizontal-tb;font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="writing-mode:horizontal-tb;font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="writing-mode:horizontal-tb;font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (12.0, 18.0, 34.0, 18.0)),
            ("a2", (51.0, 10.0, 36.0, 24.0)),
            ("a3", (89.0, 11.0, 36.0, 19.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_horizontal_items_baseline() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-rl;justify-items:baseline"><div id="a1" style="writing-mode:horizontal-tb;font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="writing-mode:horizontal-tb;font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="writing-mode:horizontal-tb;font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (72.0, 18.0, 34.0, 18.0)),
            ("a2", (31.0, 10.0, 36.0, 24.0)),
            ("a3", (-11.0, 11.0, 36.0, 19.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_horizontal_vertical_lr_items_first() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;justify-items:baseline;text-orientation:sideways"><div id="a1" style="writing-mode:vertical-lr;font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="writing-mode:vertical-lr;font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="writing-mode:vertical-lr;font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 36.0)),
            ("a2", (18.0, 50.0, 18.0, 33.0)),
            ("a3", (19.0, 89.0, 25.0, 37.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_horizontal_vertical_rl_items_first() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;justify-items:baseline;text-orientation:sideways"><div id="a1" style="writing-mode:vertical-rl;font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="writing-mode:vertical-rl;font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="writing-mode:vertical-rl;font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (73.0, 11.0, 20.0, 36.0)),
            ("a2", (79.0, 50.0, 18.0, 33.0)),
            ("a3", (80.0, 89.0, 25.0, 37.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_horizontal_mixed_parallel_and_vertical_lr() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;justify-items:baseline;text-orientation:sideways"><div id="a1" style="font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="writing-mode:vertical-lr;font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (13.0, 11.0, 20.0, 36.0)),
            ("a2", (11.0, 50.0, 18.0, 33.0)),
            ("a3", (13.0, 89.0, 45.0, 37.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_mixed_parallel_and_horizontal() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;writing-mode:vertical-lr;justify-items:baseline;text-orientation:sideways"><div id="a1" style="writing-mode:vertical-lr;font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="writing-mode:vertical-lr;font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (12.0, 30.0, 34.0, 18.0)),
            ("a2", (51.0, 14.0, 36.0, 34.0)),
            ("a3", (89.0, 9.0, 36.0, 39.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_horizontal_vertical_rl_item_packs_right() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;justify-items:baseline;text-orientation:sideways"><div id="a1" style="font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="writing-mode:vertical-rl;font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 36.0)),
            ("a2", (89.0, 50.0, 18.0, 33.0)),
            ("a3", (12.0, 89.0, 45.0, 37.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_two_columns_share_per_column() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:50px 50px;grid-template-rows:30px 30px;width:100px;height:60px;justify-items:baseline;align-content:start;justify-content:start"><div id="a1" style="width:20px;height:10px;margin:0 0 0 4px"></div><div id="a2" style="width:20px;height:10px;margin:0 0 0 7px"></div><div id="a3" style="width:20px;height:10px;margin:0 0 0 9px"></div><div id="a4" style="width:20px;height:10px;margin:0 0 0 1px"></div></div>"#,
        &[
            ("a1", (17.0, 8.0, 20.0, 10.0)),
            ("a2", (65.0, 8.0, 20.0, 10.0)),
            ("a3", (17.0, 38.0, 20.0, 10.0)),
            ("a4", (65.0, 38.0, 20.0, 10.0)),
            ("g", (8.0, 8.0, 100.0, 60.0)),
        ],
    );
}

#[test]
fn grid_justify_self_on_items() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;justify-self:baseline"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;justify-self:baseline"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;justify-self:end"></div></div>"#,
        &[
            ("a1", (15.0, 11.0, 30.0, 20.0)),
            ("a2", (15.0, 50.0, 20.0, 20.0)),
            ("a3", (80.0, 89.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_auto_margin_item_does_not_participate() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px 40px;width:100px;height:140px;align-content:start;justify-content:start;justify-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px"></div><div id="a2" style="width:20px;height:20px;margin:1px auto 5px 7px"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px"></div></div>"#,
        &[
            ("a1", (12.0, 11.0, 30.0, 20.0)),
            ("a2", (15.0, 49.0, 20.0, 20.0)),
            ("a3", (12.0, 89.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_horizontal_tb_parallel_baseline() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:baseline;writing-mode:horizontal-tb;direction:ltr"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (15.0, 11.0, 30.0, 20.0)),
            ("a2", (15.0, 34.0, 20.0, 20.0)),
            ("a3", (15.0, 60.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_horizontal_tb_parallel_last() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:last baseline;writing-mode:horizontal-tb;direction:ltr"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 11.0, 30.0, 20.0)),
            ("a2", (76.0, 34.0, 20.0, 20.0)),
            ("a3", (76.0, 60.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_vertical_lr_parallel_baseline() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:baseline;writing-mode:vertical-lr;direction:ltr"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (12.0, 11.0, 30.0, 20.0)),
            ("a2", (51.0, 11.0, 20.0, 20.0)),
            ("a3", (73.0, 11.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_vertical_lr_parallel_last() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:last baseline;writing-mode:vertical-lr;direction:ltr"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (12.0, 123.0, 30.0, 20.0)),
            ("a2", (51.0, 123.0, 20.0, 20.0)),
            ("a3", (73.0, 123.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_vertical_rl_parallel_baseline() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:baseline;writing-mode:vertical-rl;direction:ltr"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 11.0, 30.0, 20.0)),
            ("a2", (51.0, 11.0, 20.0, 20.0)),
            ("a3", (16.0, 11.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_vertical_rl_parallel_last() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:last baseline;writing-mode:vertical-rl;direction:ltr"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 123.0, 30.0, 20.0)),
            ("a2", (51.0, 123.0, 20.0, 20.0)),
            ("a3", (16.0, 123.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_rtl_parallel_first() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:baseline;direction:rtl"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (76.0, 11.0, 30.0, 20.0)),
            ("a2", (76.0, 34.0, 20.0, 20.0)),
            ("a3", (76.0, 60.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_vertical_lr_horizontal_items() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:horizontal-tb;font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="writing-mode:horizontal-tb;font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="writing-mode:horizontal-tb;font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (12.0, 18.0, 20.0, 18.0)),
            ("a2", (37.0, 10.0, 28.0, 24.0)),
            ("a3", (67.0, 11.0, 45.0, 19.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_horizontal_vertical_lr_items() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:baseline;text-orientation:sideways"><div id="a1" style="writing-mode:vertical-lr;font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="writing-mode:vertical-lr;font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="writing-mode:vertical-lr;font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (18.0, 32.0, 18.0, 34.0)),
            ("a3", (19.0, 72.0, 25.0, 39.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_row_still_aligns_by_block_axis_baseline() {
    check(
        r#"<div id="g" style="display:flex;width:200px;height:60px;align-items:baseline"><div id="a1" style="font-size:10px;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">É</div><div id="a2" style="font-size:20px;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">É</div><div id="a3" style="font-size:30px;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">É</div></div>"#,
        &[
            ("a1", (12.0, 18.0, 20.0, 18.0)),
            ("a2", (37.0, 10.0, 28.0, 24.0)),
            ("a3", (67.0, 11.0, 45.0, 19.0)),
            ("g", (8.0, 8.0, 200.0, 60.0)),
        ],
    );
}

#[test]
fn grid_two_columns_rtl() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:50px 50px;grid-template-rows:30px 30px;width:100px;height:60px;justify-items:baseline;align-content:start;justify-content:start;direction:rtl"><div id="a1" style="width:20px;height:10px;margin:0 0 0 4px"></div><div id="a2" style="width:20px;height:10px;margin:0 0 0 7px"></div><div id="a3" style="width:20px;height:10px;margin:0 0 0 9px"></div><div id="a4" style="width:20px;height:10px;margin:0 0 0 1px"></div></div>"#,
        &[
            ("a1", (88.0, 8.0, 20.0, 10.0)),
            ("a2", (38.0, 8.0, 20.0, 10.0)),
            ("a3", (88.0, 38.0, 20.0, 10.0)),
            ("a4", (38.0, 38.0, 20.0, 10.0)),
            ("g", (8.0, 8.0, 100.0, 60.0)),
        ],
    );
}

#[test]
fn grid_two_columns_rtl_last() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:50px 50px;grid-template-rows:30px 30px;width:100px;height:60px;justify-items:last baseline;align-content:start;justify-content:start;direction:rtl"><div id="a1" style="width:20px;height:10px;margin:0 0 0 4px"></div><div id="a2" style="width:20px;height:10px;margin:0 0 0 7px"></div><div id="a3" style="width:20px;height:10px;margin:0 0 0 9px"></div><div id="a4" style="width:20px;height:10px;margin:0 0 0 1px"></div></div>"#,
        &[
            ("a1", (67.0, 8.0, 20.0, 10.0)),
            ("a2", (15.0, 8.0, 20.0, 10.0)),
            ("a3", (67.0, 38.0, 20.0, 10.0)),
            ("a4", (15.0, 38.0, 20.0, 10.0)),
            ("g", (8.0, 8.0, 100.0, 60.0)),
        ],
    );
}

#[test]
fn grid_two_columns_vertical_lr() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:50px 50px;grid-template-rows:30px 30px;width:60px;height:100px;justify-items:baseline;align-content:start;justify-content:start;writing-mode:vertical-lr"><div id="a1" style="width:20px;height:10px;margin:4px 0 0 0"></div><div id="a2" style="width:20px;height:10px;margin:7px 0 0 0"></div><div id="a3" style="width:20px;height:10px;margin:9px 0 0 0"></div><div id="a4" style="width:20px;height:10px;margin:1px 0 0 0"></div></div>"#,
        &[
            ("a1", (8.0, 17.0, 20.0, 10.0)),
            ("a2", (8.0, 65.0, 20.0, 10.0)),
            ("a3", (38.0, 17.0, 20.0, 10.0)),
            ("a4", (38.0, 65.0, 20.0, 10.0)),
            ("g", (8.0, 8.0, 60.0, 100.0)),
        ],
    );
}

#[test]
fn grid_two_columns_vertical_rl() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:50px 50px;grid-template-rows:30px 30px;width:60px;height:100px;justify-items:baseline;align-content:start;justify-content:start;writing-mode:vertical-rl"><div id="a1" style="width:20px;height:10px;margin:4px 0 0 0"></div><div id="a2" style="width:20px;height:10px;margin:7px 0 0 0"></div><div id="a3" style="width:20px;height:10px;margin:9px 0 0 0"></div><div id="a4" style="width:20px;height:10px;margin:1px 0 0 0"></div></div>"#,
        &[
            ("a1", (48.0, 17.0, 20.0, 10.0)),
            ("a2", (48.0, 65.0, 20.0, 10.0)),
            ("a3", (18.0, 17.0, 20.0, 10.0)),
            ("a4", (18.0, 65.0, 20.0, 10.0)),
            ("g", (8.0, 8.0, 60.0, 100.0)),
        ],
    );
}

#[test]
fn grid_padding_border_and_content_alignment() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:40px 40px;grid-template-rows:30px 30px;column-gap:6px;width:140px;height:90px;padding:3px 5px 7px 11px;border:2px solid;justify-items:baseline;justify-content:center;align-content:start"><div id="a1" style="width:20px;height:10px;margin:0 0 0 4px"></div><div id="a2" style="width:20px;height:10px;margin:0 0 0 7px"></div><div id="a3" style="width:20px;height:10px;margin:0 0 0 9px"></div><div id="a4" style="width:20px;height:10px;margin:0 0 0 1px"></div></div>"#,
        &[
            ("a1", (47.0, 13.0, 20.0, 10.0)),
            ("a2", (91.0, 13.0, 20.0, 10.0)),
            ("a3", (47.0, 43.0, 20.0, 10.0)),
            ("a4", (91.0, 43.0, 20.0, 10.0)),
            ("g", (8.0, 8.0, 140.0, 90.0)),
        ],
    );
}

#[test]
fn grid_spanning_item_joins_first_column_group() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:50px 50px;grid-template-rows:30px 30px 30px;width:100px;height:90px;justify-items:baseline;align-content:start;justify-content:start"><div id="a1" style="width:20px;height:10px;margin:0 0 0 4px;grid-column:1/3"></div><div id="a2" style="width:20px;height:10px;margin:0 0 0 9px;grid-column:1;grid-row:2"></div><div id="a3" style="width:20px;height:10px;margin:0 0 0 2px;grid-column:2;grid-row:2"></div></div>"#,
        &[
            ("a1", (17.0, 8.0, 20.0, 10.0)),
            ("a2", (17.0, 38.0, 20.0, 10.0)),
            ("a3", (60.0, 38.0, 20.0, 10.0)),
            ("g", (8.0, 8.0, 100.0, 90.0)),
        ],
    );
}

#[test]
fn grid_spanning_last_baseline_joins_last_column_group() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:50px 50px;grid-template-rows:30px 30px 30px;width:100px;height:90px;justify-items:baseline;align-content:start;justify-content:start;justify-items:last baseline"><div id="a1" style="width:20px;height:10px;margin:0 4px 0 0;grid-column:1/3;grid-row:2"></div><div id="a2" style="width:20px;height:10px;margin:0 9px 0 0;grid-column:2;grid-row:1"></div></div>"#,
        &[
            ("a1", (79.0, 38.0, 20.0, 10.0)),
            ("a2", (79.0, 8.0, 20.0, 10.0)),
            ("g", (8.0, 8.0, 100.0, 90.0)),
        ],
    );
}

#[test]
fn flex_column_padding_and_border() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;padding:3px 5px 7px 11px;border:2px solid;align-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (28.0, 16.0, 30.0, 20.0)),
            ("a2", (28.0, 39.0, 20.0, 20.0)),
            ("a3", (28.0, 65.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_reverse_aligns_the_same() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column-reverse;width:100px;height:140px;align-items:baseline"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px;"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;"></div></div>"#,
        &[
            ("a1", (15.0, 127.0, 30.0, 20.0)),
            ("a2", (15.0, 99.0, 20.0, 20.0)),
            ("a3", (15.0, 75.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn flex_column_align_self_mixed() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:140px;align-items:flex-end"><div id="a1" style="width:30px;height:20px;margin:3px 2px 1px 4px;align-self:baseline"></div><div id="a2" style="width:20px;height:20px;margin:2px 1px 5px 7px"></div><div id="a3" style="width:25px;height:20px;margin:1px 3px 2px 1px;align-self:baseline"></div></div>"#,
        &[
            ("a1", (12.0, 11.0, 30.0, 20.0)),
            ("a2", (87.0, 34.0, 20.0, 20.0)),
            ("a3", (12.0, 60.0, 25.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 140.0)),
        ],
    );
}

#[test]
fn grid_relative_item_keeps_its_offset() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px;width:100px;height:80px;justify-items:baseline;align-content:start;justify-content:start;"><div id="a1" style="width:30px;height:20px;margin:0 0 0 5px;position:relative;left:10px"></div><div id="a2" style="width:20px;height:20px;margin:0 0 0 12px"></div></div>"#,
        &[
            ("a1", (30.0, 8.0, 30.0, 20.0)),
            ("a2", (20.0, 48.0, 20.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 80.0)),
        ],
    );
}

#[test]
fn grid_relative_item_keeps_its_offset_from_the_right() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px;width:100px;height:80px;justify-items:baseline;align-content:start;justify-content:start;"><div id="a1" style="width:30px;height:20px;margin:0 0 0 5px;position:relative;right:10px"></div><div id="a2" style="width:20px;height:20px;margin:0 0 0 12px"></div></div>"#,
        &[
            ("a1", (10.0, 8.0, 30.0, 20.0)),
            ("a2", (20.0, 48.0, 20.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 80.0)),
        ],
    );
}

#[test]
fn grid_relative_item_keeps_its_offset_rtl() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px;width:100px;height:80px;justify-items:baseline;align-content:start;justify-content:start;direction:rtl"><div id="a1" style="width:30px;height:20px;margin:0 0 0 5px;position:relative;left:10px"></div><div id="a2" style="width:20px;height:20px;margin:0 0 0 12px"></div></div>"#,
        &[
            ("a1", (88.0, 8.0, 30.0, 20.0)),
            ("a2", (78.0, 48.0, 20.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 80.0)),
        ],
    );
}

#[test]
fn grid_relative_item_keeps_its_offset_vertical() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px;grid-template-rows:40px 40px;width:80px;height:100px;justify-items:baseline;align-content:start;justify-content:start;writing-mode:vertical-lr"><div id="a1" style="width:20px;height:30px;margin:5px 0 0 0;position:relative;top:10px"></div><div id="a2" style="width:20px;height:20px;margin:12px 0 0 0"></div></div>"#,
        &[
            ("a1", (8.0, 25.0, 20.0, 30.0)),
            ("a2", (48.0, 20.0, 20.0, 20.0)),
            ("g", (8.0, 8.0, 80.0, 100.0)),
        ],
    );
}

#[test]
fn flex_column_relative_item_keeps_its_offset() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:80px;align-items:baseline;"><div id="a1" style="width:30px;height:20px;margin:0 0 0 5px;position:relative;left:10px"></div><div id="a2" style="width:20px;height:20px;margin:0 0 0 12px"></div></div>"#,
        &[
            ("a1", (30.0, 8.0, 30.0, 20.0)),
            ("a2", (20.0, 28.0, 20.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 80.0)),
        ],
    );
}

#[test]
fn flex_column_relative_item_keeps_its_offset_rtl() {
    check(
        r#"<div id="g" style="display:flex;flex-direction:column;width:100px;height:80px;align-items:baseline;direction:rtl"><div id="a1" style="width:30px;height:20px;margin:0 0 0 5px;position:relative;left:10px"></div><div id="a2" style="width:20px;height:20px;margin:0 0 0 12px"></div></div>"#,
        &[
            ("a1", (88.0, 8.0, 30.0, 20.0)),
            ("a2", (78.0, 28.0, 20.0, 20.0)),
            ("g", (8.0, 8.0, 100.0, 80.0)),
        ],
    );
}
