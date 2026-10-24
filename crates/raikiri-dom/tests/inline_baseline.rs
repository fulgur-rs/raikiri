//! Baseline alignment that the flex and grid algorithms leave to the
//! container: along the inline axis, by grid `justify-self` and
//! `justify-items` and in the cross axis of a column flex container, and along
//! the block axis of a vertical flex row or grid, for the items whose block
//! flow runs against the container's.
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

#[test]
fn grid_static_position_of_an_abspos_follows_its_moved_item() {
    check(
        r#"<div id=g style="position:relative;display:grid;grid-template-columns:100px;width:100px"><div id=a style="justify-self:baseline;margin-left:30px;width:20px;height:20px"></div><div id=b style="justify-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (38.0, 8.0, 20.0, 20.0)),
            ("b", (38.0, 28.0, 20.0, 20.0)),
            ("bb", (38.0, 28.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn grid_static_position_of_an_abspos_follows_its_moved_item_last_baseline() {
    check(
        r#"<div id=g style="position:relative;display:grid;grid-template-columns:100px;width:100px"><div id=a style="justify-self:last baseline;margin-right:30px;width:20px;height:20px"></div><div id=b style="justify-self:last baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (58.0, 8.0, 20.0, 20.0)),
            ("b", (58.0, 28.0, 20.0, 20.0)),
            ("bb", (58.0, 28.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn flex_column_static_position_of_an_abspos_follows_its_moved_item() {
    check(
        r#"<div id=g style="position:relative;display:flex;flex-direction:column;width:100px"><div id=a style="align-self:baseline;margin-left:30px;width:20px;height:20px"></div><div id=b style="align-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (38.0, 8.0, 20.0, 20.0)),
            ("b", (38.0, 28.0, 20.0, 20.0)),
            ("bb", (38.0, 28.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn grid_static_position_of_a_nested_abspos_follows_its_moved_item() {
    check(
        r#"<div id=g style="position:relative;display:grid;grid-template-columns:100px;width:100px"><div id=a style="justify-self:baseline;margin-left:30px;width:20px;height:20px"></div><div id=b style="justify-self:baseline;width:20px;height:20px"><div id=m style="padding:3px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div></div>"#,
        &[
            ("a", (38.0, 8.0, 20.0, 20.0)),
            ("b", (38.0, 28.0, 20.0, 20.0)),
            ("bb", (41.0, 31.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 100.0, 40.0)),
            ("m", (38.0, 28.0, 20.0, 6.0)),
        ],
    );
}

#[test]
fn grid_abspos_in_a_positioned_moved_item_stays_in_the_item() {
    check(
        r#"<div id=g style="display:grid;grid-template-columns:100px;width:100px"><div id=a style="justify-self:baseline;margin-left:30px;width:20px;height:20px"></div><div id=b style="position:relative;justify-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (38.0, 8.0, 20.0, 20.0)),
            ("b", (38.0, 28.0, 20.0, 20.0)),
            ("bb", (38.0, 28.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn grid_static_position_of_an_abspos_follows_its_moved_item_vertical() {
    check(
        r#"<div id=g style="position:relative;display:grid;writing-mode:vertical-rl;grid-template-columns:100px;height:100px"><div id=a style="justify-self:baseline;margin-top:30px;width:20px;height:20px"></div><div id=b style="justify-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (28.0, 38.0, 20.0, 20.0)),
            ("b", (8.0, 38.0, 20.0, 20.0)),
            ("bb", (23.0, 38.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 40.0, 100.0)),
        ],
    );
}

#[test]
fn grid_static_position_of_an_abspos_follows_its_moved_item_to_a_distant_containing_block() {
    check(
        r#"<div id=w style="position:relative;margin-left:5px"><div id=g style="display:grid;grid-template-columns:100px;width:100px"><div id=a style="justify-self:baseline;margin-left:30px;width:20px;height:20px"></div><div id=b style="justify-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div></div>"#,
        &[
            ("a", (43.0, 8.0, 20.0, 20.0)),
            ("b", (43.0, 28.0, 20.0, 20.0)),
            ("bb", (43.0, 28.0, 5.0, 5.0)),
            ("g", (13.0, 8.0, 100.0, 40.0)),
            ("w", (13.0, 8.0, 779.0, 40.0)),
        ],
    );
}

#[test]
fn flex_column_static_position_of_an_abspos_follows_its_moved_item_to_a_distant_containing_block() {
    check(
        r#"<div id=w style="position:relative;margin-left:5px"><div id=g style="display:flex;flex-direction:column;width:100px"><div id=a style="align-self:baseline;margin-left:30px;width:20px;height:20px"></div><div id=b style="align-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div></div>"#,
        &[
            ("a", (43.0, 8.0, 20.0, 20.0)),
            ("b", (43.0, 28.0, 20.0, 20.0)),
            ("bb", (43.0, 28.0, 5.0, 5.0)),
            ("g", (13.0, 8.0, 100.0, 40.0)),
            ("w", (13.0, 8.0, 779.0, 40.0)),
        ],
    );
}

#[test]
fn grid_abspos_anchored_by_an_inset_does_not_follow_its_moved_item() {
    check(
        r#"<div id=g style="position:relative;display:grid;grid-template-columns:100px;width:100px"><div id=a style="justify-self:baseline;margin-left:30px;width:20px;height:20px"></div><div id=b style="justify-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;left:7px;top:auto;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (38.0, 8.0, 20.0, 20.0)),
            ("b", (38.0, 28.0, 20.0, 20.0)),
            ("bb", (15.0, 28.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn flex_column_static_position_of_an_abspos_follows_its_moved_item_rtl() {
    check(
        r#"<div id=g style="position:relative;display:flex;flex-direction:column;direction:rtl;width:100px"><div id=a style="align-self:baseline;margin-right:30px;width:20px;height:20px"></div><div id=b style="align-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (58.0, 8.0, 20.0, 20.0)),
            ("b", (58.0, 28.0, 20.0, 20.0)),
            ("bb", (73.0, 28.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn flex_column_static_position_of_an_abspos_follows_its_moved_item_vertical() {
    check(
        r#"<div id=g style="position:relative;display:flex;flex-direction:column;writing-mode:vertical-rl;height:100px"><div id=a style="align-self:baseline;margin-top:30px;width:20px;height:20px"></div><div id=b style="align-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (28.0, 38.0, 20.0, 20.0)),
            ("b", (8.0, 38.0, 20.0, 20.0)),
            ("bb", (23.0, 38.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 40.0, 100.0)),
        ],
    );
}

#[test]
fn grid_static_positions_follow_each_of_the_moved_items() {
    check(
        r#"<div id=g style="position:relative;display:grid;grid-template-columns:100px;width:100px"><div id=a style="justify-self:baseline;margin-left:30px;width:20px;height:20px"><div id=aa style="position:absolute;width:5px;height:5px"></div></div><div id=b style="justify-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (38.0, 8.0, 20.0, 20.0)),
            ("aa", (38.0, 8.0, 5.0, 5.0)),
            ("b", (38.0, 28.0, 20.0, 20.0)),
            ("bb", (38.0, 28.0, 5.0, 5.0)),
            ("g", (8.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn grid_static_position_of_an_abspos_follows_its_moved_item_to_a_distant_containing_block_vertical()
{
    check(
        r#"<div id=w style="position:relative;margin-left:5px;writing-mode:vertical-rl"><div id=g style="display:grid;grid-template-columns:100px;height:100px"><div id=a style="justify-self:baseline;margin-top:30px;width:20px;height:20px"></div><div id=b style="justify-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div></div>"#,
        &[
            ("a", (33.0, 38.0, 20.0, 20.0)),
            ("b", (13.0, 38.0, 20.0, 20.0)),
            ("bb", (28.0, 38.0, 5.0, 5.0)),
            ("g", (13.0, 8.0, 40.0, 100.0)),
            ("w", (13.0, 8.0, 40.0, 100.0)),
        ],
    );
}

#[test]
fn flex_column_static_position_of_an_abspos_follows_its_moved_item_to_a_distant_containing_block_vertical()
 {
    check(
        r#"<div id=w style="position:relative;margin-left:5px;writing-mode:vertical-rl"><div id=g style="display:flex;flex-direction:column;height:100px"><div id=a style="align-self:baseline;margin-top:30px;width:20px;height:20px"></div><div id=b style="align-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div></div></div>"#,
        &[
            ("a", (33.0, 38.0, 20.0, 20.0)),
            ("b", (13.0, 38.0, 20.0, 20.0)),
            ("bb", (28.0, 38.0, 5.0, 5.0)),
            ("g", (13.0, 8.0, 40.0, 100.0)),
            ("w", (13.0, 8.0, 40.0, 100.0)),
        ],
    );
}

#[test]
fn grid_static_position_of_a_direct_abspos_child_stays_while_items_move() {
    check(
        r#"<div id=w style="position:relative;margin-left:5px"><div id=g style="display:grid;grid-template-columns:100px;width:100px"><div id=a style="justify-self:baseline;margin-left:30px;width:20px;height:20px"></div><div id=b style="justify-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div><div id=c style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (43.0, 8.0, 20.0, 20.0)),
            ("b", (43.0, 28.0, 20.0, 20.0)),
            ("bb", (43.0, 28.0, 5.0, 5.0)),
            ("c", (13.0, 8.0, 5.0, 5.0)),
            ("g", (13.0, 8.0, 100.0, 40.0)),
            ("w", (13.0, 8.0, 779.0, 40.0)),
        ],
    );
}

#[test]
fn flex_column_static_position_of_a_direct_abspos_child_stays_while_items_move() {
    check(
        r#"<div id=w style="position:relative;margin-left:5px"><div id=g style="display:flex;flex-direction:column;width:100px"><div id=a style="align-self:baseline;margin-left:30px;width:20px;height:20px"></div><div id=b style="align-self:baseline;width:20px;height:20px"><div id=bb style="position:absolute;width:5px;height:5px"></div></div><div id=c style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a", (43.0, 8.0, 20.0, 20.0)),
            ("b", (43.0, 28.0, 20.0, 20.0)),
            ("bb", (43.0, 28.0, 5.0, 5.0)),
            ("c", (13.0, 8.0, 5.0, 5.0)),
            ("g", (13.0, 8.0, 100.0, 40.0)),
            ("w", (13.0, 8.0, 779.0, 40.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_opposed_items_share_a_baseline_group() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 18.0)),
            ("a2", (11.0, 32.0, 18.0, 24.0)),
            ("a3", (9.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_opposed_items_share_a_baseline_group_auto_cross_size() {
    check(
        r#"<div id="g" style="display:flex;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (15.0, 11.0, 20.0, 18.0)),
            ("a2", (11.0, 32.0, 18.0, 24.0)),
            ("a3", (9.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 29.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_opposed_last_baselines_pack_against_the_other_edge() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:last baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (129.0, 11.0, 20.0, 18.0)),
            ("a2", (132.0, 32.0, 18.0, 24.0)),
            ("a3", (130.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_opposed_and_horizontal_items() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:horizontal-tb;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (131.5, 11.0, 20.0, 18.0)),
            ("a2", (11.0, 32.0, 18.0, 24.0)),
            ("a3", (130.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_empty_opposed_item_has_a_central_baseline() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;height:20px"></div><div id="a3" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 18.0)),
            ("a2", (19.0, 32.0, 8.0, 20.0)),
            ("a3", (12.0, 60.0, 20.0, 18.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_empty_opposed_items_have_alphabetic_baselines_in_sideways_text() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl;text-orientation:sideways"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;height:20px"></div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;height:20px"></div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;height:30px"></div></div>"#,
        &[
            ("a1", (146.0, 11.0, 10.0, 20.0)),
            ("a2", (11.0, 34.0, 8.0, 20.0)),
            ("a3", (11.0, 60.0, 15.0, 30.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_two_same_direction_and_three_opposed_items() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="a4" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a5" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;margin-left:7px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 18.0)),
            ("a2", (18.0, 32.0, 18.0, 24.0)),
            ("a3", (16.0, 62.0, 25.0, 19.0)),
            ("a4", (139.0, 85.0, 18.0, 24.0)),
            ("a5", (15.0, 117.0, 20.0, 18.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_container_padding_and_border() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;padding:5px 7px 9px 11px;border:3px solid;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (126.0, 19.0, 20.0, 18.0)),
            ("a2", (25.0, 40.0, 18.0, 24.0)),
            ("a3", (23.0, 70.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_relative_opposed_items_keep_their_offsets() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;position:relative;left:3px;top:2px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;position:relative;right:4px;bottom:3px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 18.0)),
            ("a2", (14.0, 34.0, 18.0, 24.0)),
            ("a3", (5.0, 59.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_align_self_baseline_among_centered_items() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:center;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;align-self:center">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;align-self:baseline">X</div></div>"#,
        &[
            ("a1", (74.0, 11.0, 20.0, 18.0)),
            ("a2", (75.0, 32.0, 18.0, 24.0)),
            ("a3", (9.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_opposed_item_aligned_to_end_does_not_share() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;align-self:end">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 18.0)),
            ("a2", (11.0, 32.0, 18.0, 24.0)),
            ("a3", (9.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_opposed_items_of_a_definite_block_size() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;width:50px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;width:40px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;width:30px">X</div></div>"#,
        &[
            ("a1", (106.0, 11.0, 50.0, 18.0)),
            ("a2", (11.0, 32.0, 40.0, 24.0)),
            ("a3", (9.0, 62.0, 30.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_opposed_item_with_an_auto_margin_does_not_participate() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;margin-left:auto">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 18.0)),
            ("a2", (139.0, 32.0, 18.0, 24.0)),
            ("a3", (9.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_opposed_scroll_container_baseline_is_clamped() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;overflow:hidden;height:15px">XX</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 18.0)),
            ("a2", (11.0, 32.0, 18.0, 15.0)),
            ("a3", (9.0, 53.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_inline_direction_rtl() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;direction:rtl;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 289.0, 20.0, 18.0)),
            ("a2", (11.0, 257.0, 18.0, 24.0)),
            ("a3", (9.0, 234.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_static_positions_follow_the_moved_opposed_item() {
    check(
        r#"<div style="position:relative"><div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;position:relative;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;width:20px;height:20px"><div id="p" style="position:absolute;width:5px;height:5px"></div></div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="q" style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 18.0)),
            ("a2", (11.0, 32.0, 20.0, 20.0)),
            ("a3", (11.0, 58.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
            ("p", (14.0, 40.0, 5.0, 5.0)),
            ("q", (153.0, 8.0, 5.0, 5.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_opposed_items_share_a_baseline_group() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (132.0, 32.0, 18.0, 24.0)),
            ("a3", (130.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_opposed_items_share_a_baseline_group_auto_cross_size() {
    check(
        r#"<div id="g" style="display:flex;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (11.0, 32.0, 18.0, 24.0)),
            ("a3", (9.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 29.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_opposed_last_baselines_pack_against_the_other_edge() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:last baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (15.0, 32.0, 18.0, 24.0)),
            ("a3", (13.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_opposed_and_horizontal_items() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:horizontal-tb;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (139.0, 32.0, 18.0, 24.0)),
            ("a3", (10.5, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_empty_opposed_item_has_a_central_baseline() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;height:20px"></div><div id="a3" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (143.0, 32.0, 8.0, 20.0)),
            ("a3", (136.0, 60.0, 20.0, 18.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_empty_opposed_items_have_alphabetic_baselines_in_sideways_text() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr;text-orientation:sideways"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;height:20px"></div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;height:20px"></div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;height:30px"></div></div>"#,
        &[
            ("a1", (12.0, 11.0, 10.0, 20.0)),
            ("a2", (140.0, 34.0, 8.0, 20.0)),
            ("a3", (140.0, 60.0, 15.0, 30.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_two_same_direction_and_three_opposed_items() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="a4" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a5" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;margin-left:7px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (132.0, 32.0, 18.0, 24.0)),
            ("a3", (130.0, 62.0, 25.0, 19.0)),
            ("a4", (15.0, 85.0, 18.0, 24.0)),
            ("a5", (129.0, 117.0, 20.0, 18.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_container_padding_and_border() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;padding:5px 7px 9px 11px;border:3px solid;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (26.0, 19.0, 20.0, 18.0)),
            ("a2", (122.0, 40.0, 18.0, 24.0)),
            ("a3", (120.0, 70.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_relative_opposed_items_keep_their_offsets() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;position:relative;left:3px;top:2px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;position:relative;right:4px;bottom:3px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (135.0, 34.0, 18.0, 24.0)),
            ("a3", (126.0, 59.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_align_self_baseline_among_centered_items() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:center;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;align-self:center">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;align-self:baseline">X</div></div>"#,
        &[
            ("a1", (74.0, 11.0, 20.0, 18.0)),
            ("a2", (75.0, 32.0, 18.0, 24.0)),
            ("a3", (130.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_opposed_item_aligned_to_end_does_not_share() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;align-self:end">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (139.0, 32.0, 18.0, 24.0)),
            ("a3", (130.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_opposed_items_of_a_definite_block_size() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;width:50px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;width:40px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;width:30px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 50.0, 18.0)),
            ("a2", (110.0, 32.0, 40.0, 24.0)),
            ("a3", (125.0, 62.0, 30.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_opposed_item_with_an_auto_margin_does_not_participate() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;margin-left:auto">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (139.0, 32.0, 18.0, 24.0)),
            ("a3", (130.0, 62.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_opposed_scroll_container_baseline_is_clamped() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;overflow:hidden;height:15px">XX</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (132.0, 32.0, 18.0, 15.0)),
            ("a3", (130.0, 53.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_inline_direction_rtl() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;direction:rtl;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 289.0, 20.0, 18.0)),
            ("a2", (132.0, 257.0, 18.0, 24.0)),
            ("a3", (130.0, 234.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_static_positions_follow_the_moved_opposed_item() {
    check(
        r#"<div style="position:relative"><div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;position:relative;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;width:20px;height:20px"><div id="p" style="position:absolute;width:5px;height:5px"></div></div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="q" style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (130.0, 32.0, 20.0, 20.0)),
            ("a3", (130.0, 58.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
            ("p", (140.0, 40.0, 5.0, 5.0)),
            ("q", (8.0, 8.0, 5.0, 5.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_items_share_a_baseline_group() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 96.0)),
            ("a2", (11.0, 110.0, 18.0, 93.0)),
            ("a3", (9.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_items_share_a_baseline_group_auto_cross_size() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (15.0, 11.0, 20.0, 96.0)),
            ("a2", (11.0, 110.0, 18.0, 93.0)),
            ("a3", (9.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 29.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_last_baselines_pack_against_the_other_edge() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:last baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (129.0, 11.0, 20.0, 96.0)),
            ("a2", (132.0, 110.0, 18.0, 93.0)),
            ("a3", (130.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_and_horizontal_items() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:horizontal-tb;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (131.5, 11.0, 20.0, 96.0)),
            ("a2", (11.0, 110.0, 18.0, 93.0)),
            ("a3", (130.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_empty_opposed_item_has_a_central_baseline() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;height:20px"></div><div id="a3" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 96.0)),
            ("a2", (19.0, 110.0, 8.0, 20.0)),
            ("a3", (12.0, 211.0, 20.0, 96.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_empty_opposed_items_have_alphabetic_baselines_in_sideways_text() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl;text-orientation:sideways"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;height:20px"></div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;height:20px"></div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;height:30px"></div></div>"#,
        &[
            ("a1", (146.0, 11.0, 10.0, 20.0)),
            ("a2", (11.0, 110.0, 8.0, 20.0)),
            ("a3", (11.0, 209.0, 15.0, 30.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_two_same_direction_and_three_opposed_items() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="a4" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a5" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;margin-left:7px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 96.0)),
            ("a2", (86.0, 110.0, 18.0, 93.0)),
            ("a3", (84.0, 209.0, 25.0, 97.0)),
            ("a4", (64.0, 10.0, 18.0, 93.0)),
            ("a5", (15.0, 111.0, 20.0, 96.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_container_padding_and_border() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;padding:5px 7px 9px 11px;border:3px solid;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (126.0, 19.0, 20.0, 96.0)),
            ("a2", (25.0, 118.0, 18.0, 93.0)),
            ("a3", (23.0, 217.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_relative_opposed_items_keep_their_offsets() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;position:relative;left:3px;top:2px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;position:relative;right:4px;bottom:3px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 96.0)),
            ("a2", (14.0, 112.0, 18.0, 93.0)),
            ("a3", (5.0, 206.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_align_self_baseline_among_centered_items() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:center;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;align-self:center">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;align-self:baseline">X</div></div>"#,
        &[
            ("a1", (74.0, 11.0, 20.0, 96.0)),
            ("a2", (75.0, 110.0, 18.0, 93.0)),
            ("a3", (9.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_item_aligned_to_end_does_not_share() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;align-self:end">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 96.0)),
            ("a2", (11.0, 110.0, 18.0, 93.0)),
            ("a3", (9.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_items_of_a_definite_block_size() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;width:50px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;width:40px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;width:30px">X</div></div>"#,
        &[
            ("a1", (106.0, 11.0, 50.0, 96.0)),
            ("a2", (11.0, 110.0, 40.0, 93.0)),
            ("a3", (9.0, 209.0, 30.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_item_with_an_auto_margin_does_not_participate() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;margin-left:auto">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 96.0)),
            ("a2", (139.0, 110.0, 18.0, 93.0)),
            ("a3", (9.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_scroll_container_baseline_is_clamped() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;overflow:hidden;height:15px">XX</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 96.0)),
            ("a2", (11.0, 110.0, 18.0, 15.0)),
            ("a3", (9.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_inline_direction_rtl() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;direction:rtl;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 211.0, 20.0, 96.0)),
            ("a2", (11.0, 110.0, 18.0, 93.0)),
            ("a3", (9.0, 9.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_static_positions_follow_the_moved_opposed_item() {
    check(
        r#"<div style="position:relative"><div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;position:relative;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;width:20px;height:20px"><div id="p" style="position:absolute;width:5px;height:5px"></div></div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="q" style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a1", (136.0, 11.0, 20.0, 96.0)),
            ("a2", (11.0, 110.0, 20.0, 20.0)),
            ("a3", (11.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
            ("p", (14.0, 118.0, 5.0, 5.0)),
            ("q", (153.0, 8.0, 5.0, 5.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_items_share_a_baseline_group() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (132.0, 110.0, 18.0, 93.0)),
            ("a3", (130.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_items_share_a_baseline_group_auto_cross_size() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (11.0, 110.0, 18.0, 93.0)),
            ("a3", (9.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 29.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_last_baselines_pack_against_the_other_edge() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:last baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (15.0, 110.0, 18.0, 93.0)),
            ("a3", (13.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_and_horizontal_items() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:horizontal-tb;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (139.0, 110.0, 18.0, 93.0)),
            ("a3", (10.5, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_empty_opposed_item_has_a_central_baseline() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;height:20px"></div><div id="a3" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (143.0, 110.0, 8.0, 20.0)),
            ("a3", (136.0, 211.0, 20.0, 96.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_empty_opposed_items_have_alphabetic_baselines_in_sideways_text() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr;text-orientation:sideways"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;height:20px"></div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;height:20px"></div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;height:30px"></div></div>"#,
        &[
            ("a1", (12.0, 11.0, 10.0, 20.0)),
            ("a2", (140.0, 110.0, 8.0, 20.0)),
            ("a3", (140.0, 209.0, 15.0, 30.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_two_same_direction_and_three_opposed_items() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="a4" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a5" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;margin-left:7px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (57.0, 110.0, 18.0, 93.0)),
            ("a3", (55.0, 209.0, 25.0, 97.0)),
            ("a4", (86.0, 10.0, 18.0, 93.0)),
            ("a5", (136.0, 111.0, 20.0, 96.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_container_padding_and_border() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;padding:5px 7px 9px 11px;border:3px solid;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (26.0, 19.0, 20.0, 96.0)),
            ("a2", (122.0, 118.0, 18.0, 93.0)),
            ("a3", (120.0, 217.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_relative_opposed_items_keep_their_offsets() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;position:relative;left:3px;top:2px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;position:relative;right:4px;bottom:3px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (135.0, 112.0, 18.0, 93.0)),
            ("a3", (126.0, 206.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_align_self_baseline_among_centered_items() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:center;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;align-self:center">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;align-self:baseline">X</div></div>"#,
        &[
            ("a1", (74.0, 11.0, 20.0, 96.0)),
            ("a2", (75.0, 110.0, 18.0, 93.0)),
            ("a3", (130.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_item_aligned_to_end_does_not_share() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;align-self:end">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (139.0, 110.0, 18.0, 93.0)),
            ("a3", (130.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_items_of_a_definite_block_size() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;width:50px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;width:40px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;width:30px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 50.0, 96.0)),
            ("a2", (110.0, 110.0, 40.0, 93.0)),
            ("a3", (125.0, 209.0, 30.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_item_with_an_auto_margin_does_not_participate() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;margin-left:auto">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (139.0, 110.0, 18.0, 93.0)),
            ("a3", (130.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_scroll_container_baseline_is_clamped() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;overflow:hidden;height:15px">XX</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (132.0, 110.0, 18.0, 15.0)),
            ("a3", (130.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_inline_direction_rtl() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;direction:rtl;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 211.0, 20.0, 96.0)),
            ("a2", (132.0, 110.0, 18.0, 93.0)),
            ("a3", (130.0, 9.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_static_positions_follow_the_moved_opposed_item() {
    check(
        r#"<div style="position:relative"><div id="g" style="display:grid;grid-template-columns:100px 100px 100px;width:150px;height:300px;align-items:baseline;position:relative;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;width:20px;height:20px"><div id="p" style="position:absolute;width:5px;height:5px"></div></div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="q" style="position:absolute;width:5px;height:5px"></div></div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (130.0, 110.0, 20.0, 20.0)),
            ("a3", (130.0, 209.0, 25.0, 97.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
            ("p", (140.0, 118.0, 5.0, 5.0)),
            ("q", (8.0, 8.0, 5.0, 5.0)),
        ],
    );
}

#[test]
fn flex_vertical_rl_row_reverse() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;flex-direction:row-reverse;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (136.0, 289.0, 20.0, 18.0)),
            ("a2", (11.0, 257.0, 18.0, 24.0)),
            ("a3", (9.0, 234.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_items_share_per_row() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;grid-row:2">X</div><div id="a4" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;grid-row:2">X</div></div>"#,
        &[
            ("a1", (186.0, 11.0, 20.0, 96.0)),
            ("a2", (151.0, 110.0, 18.0, 93.0)),
            ("a3", (69.0, 9.0, 25.0, 97.0)),
            ("a4", (129.0, 110.0, 18.0, 93.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_items_share_per_auto_row() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:auto auto;width:200px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;grid-row:2">X</div><div id="a4" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;grid-row:2">X</div></div>"#,
        &[
            ("a1", (186.0, 11.0, 20.0, 96.0)),
            ("a2", (112.5, 110.0, 18.0, 93.0)),
            ("a3", (9.0, 9.0, 25.0, 97.0)),
            ("a4", (11.0, 110.0, 18.0, 93.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_spanning_opposed_item_joins_the_group_of_the_row_at_its_packing_edge() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-lr;padding-left:1px;margin-left:3px;grid-row:1/3">X</div><div id="a2" style="writing-mode:vertical-lr;padding-left:9px;margin-left:4px;grid-row:1">X</div></div>"#,
        &[
            ("a1", (71.0, 8.0, 11.0, 100.0)),
            ("a2", (152.0, 108.0, 19.0, 100.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_spanning_opposed_item_joins_the_group_of_the_other_row() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-lr;padding-left:1px;margin-left:3px;grid-row:1/3">X</div><div id="a2" style="writing-mode:vertical-lr;padding-left:9px;margin-left:4px;grid-row:2">X</div></div>"#,
        &[
            ("a1", (80.0, 8.0, 11.0, 100.0)),
            ("a2", (72.0, 108.0, 19.0, 100.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_spanning_opposed_item_packs_in_its_own_room() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;writing-mode:vertical-rl"><div id="a2" style="writing-mode:vertical-lr;padding-left:9px;margin-left:4px;grid-row:1">X</div><div id="a1" style="writing-mode:vertical-lr;padding-left:1px;margin-left:3px;grid-row:1/3">X</div><div id="a3" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;grid-row:2">X</div></div>"#,
        &[
            ("a1", (71.0, 108.0, 11.0, 100.0)),
            ("a2", (152.0, 8.0, 19.0, 100.0)),
            ("a3", (126.0, 11.0, 20.0, 96.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn flex_vertical_lr_row_reverse() {
    check(
        r#"<div id="g" style="display:flex;width:150px;height:300px;align-items:baseline;flex-direction:row-reverse;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div></div>"#,
        &[
            ("a1", (12.0, 289.0, 20.0, 18.0)),
            ("a2", (132.0, 257.0, 18.0, 24.0)),
            ("a3", (130.0, 234.0, 25.0, 19.0)),
            ("g", (8.0, 8.0, 150.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_items_share_per_row() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;grid-row:2">X</div><div id="a4" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;grid-row:2">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (49.0, 110.0, 18.0, 93.0)),
            ("a3", (120.0, 9.0, 25.0, 97.0)),
            ("a4", (71.0, 110.0, 18.0, 93.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_items_share_per_auto_row() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:auto auto;width:200px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px;grid-row:2">X</div><div id="a4" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px;grid-row:2">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 96.0)),
            ("a2", (87.5, 110.0, 18.0, 93.0)),
            ("a3", (180.0, 9.0, 25.0, 97.0)),
            ("a4", (182.0, 110.0, 18.0, 93.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_spanning_opposed_item_joins_the_group_of_the_row_at_its_packing_edge() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-rl;padding-right:1px;margin-right:3px;grid-row:1/3">X</div><div id="a2" style="writing-mode:vertical-rl;padding-right:9px;margin-right:4px;grid-row:1">X</div></div>"#,
        &[
            ("a1", (134.0, 8.0, 11.0, 100.0)),
            ("a2", (45.0, 108.0, 19.0, 100.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_spanning_opposed_item_joins_the_group_of_the_other_row() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-rl;padding-right:1px;margin-right:3px;grid-row:1/3">X</div><div id="a2" style="writing-mode:vertical-rl;padding-right:9px;margin-right:4px;grid-row:2">X</div></div>"#,
        &[
            ("a1", (125.0, 8.0, 11.0, 100.0)),
            ("a2", (125.0, 108.0, 19.0, 100.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_spanning_opposed_item_packs_in_its_own_room() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;writing-mode:vertical-lr"><div id="a2" style="writing-mode:vertical-rl;padding-right:9px;margin-right:4px;grid-row:1">X</div><div id="a1" style="writing-mode:vertical-rl;padding-right:1px;margin-right:3px;grid-row:1/3">X</div><div id="a3" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;grid-row:2">X</div></div>"#,
        &[
            ("a1", (134.0, 108.0, 11.0, 100.0)),
            ("a2", (45.0, 8.0, 19.0, 100.0)),
            ("a3", (72.0, 11.0, 20.0, 96.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_rl_opposed_items_align_along_both_axes() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;justify-items:baseline;writing-mode:vertical-rl"><div id="a1" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-lr;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-lr;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="a4" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;margin-top:9px">X</div></div>"#,
        &[
            ("a1", (186.0, 11.0, 20.0, 18.0)),
            ("a2", (151.0, 114.0, 18.0, 24.0)),
            ("a3", (73.0, 10.5, 25.0, 19.0)),
            ("a4", (72.0, 117.0, 20.0, 18.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}

#[test]
fn grid_vertical_lr_opposed_items_align_along_both_axes() {
    check(
        r#"<div id="g" style="display:grid;grid-template-columns:100px 100px;grid-template-rows:60px 80px;width:200px;height:300px;align-items:baseline;justify-items:baseline;writing-mode:vertical-lr"><div id="a1" style="writing-mode:vertical-lr;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px">X</div><div id="a2" style="writing-mode:vertical-rl;padding:6px 3px 4px 1px;border:2px solid;margin:2px 1px 5px 3px">X</div><div id="a3" style="writing-mode:vertical-rl;padding:1px 7px 2px 2px;border:3px solid;margin:1px 3px 2px 1px">X</div><div id="a4" style="writing-mode:vertical-rl;padding:2px 3px 4px 5px;border:1px solid;margin:3px 2px 1px 4px;margin-top:9px">X</div></div>"#,
        &[
            ("a1", (12.0, 11.0, 20.0, 18.0)),
            ("a2", (49.0, 114.0, 18.0, 24.0)),
            ("a3", (120.0, 10.5, 25.0, 19.0)),
            ("a4", (119.0, 117.0, 20.0, 18.0)),
            ("g", (8.0, 8.0, 200.0, 300.0)),
        ],
    );
}
