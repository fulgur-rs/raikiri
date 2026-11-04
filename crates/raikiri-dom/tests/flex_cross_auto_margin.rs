//! Cross-axis auto margins distribute positive room and never center overflow.
//! Expected border boxes were measured in Chromium at an 800x600 viewport.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

type Rect = (f32, f32, f32, f32);
type Case = (&'static str, &'static str, &'static [(&'static str, Rect)]);

fn border_boxes(markup: &str) -> Vec<(String, Rect)> {
    border_boxes_with_resources(markup, &RenderResources::new())
}

fn border_boxes_with_resources(
    markup: &str,
    resources: &RenderResources<'_>,
) -> Vec<(String, Rect)> {
    let html = format!(
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}body{{margin:0}}</style>{markup}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), resources).unwrap();
    let LayoutStatus::Completed(document) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(resources),
    )
    .unwrap() else {
        panic!("layout did not complete")
    };
    let page = document.page(0).unwrap();
    page.fragments()
        .filter_map(|f| {
            let id = page.dom().attr(f.node(), "id")?;
            let r = f.rect();
            Some((id.to_owned(), (r.x, r.y, r.width, r.height)))
        })
        .collect()
}

fn item_rect(container: &str, item: &str) -> Rect {
    border_boxes(&format!(
        "<div style='{container}'><div id=item style='{item}'></div></div>"
    ))
    .into_iter()
    .find(|(id, _)| id == "item")
    .unwrap()
    .1
}

#[test]
fn cross_auto_margins_match_chromium_for_positive_zero_and_negative_room() {
    let mut failures = Vec::new();
    for direction in ["row", "row-reverse", "column", "column-reverse"] {
        for text_direction in ["ltr", "rtl"] {
            for margin in ["both", "start", "end"] {
                for size in [100, 300, 400] {
                    let row = direction.starts_with("row");
                    let (start, end) = if row {
                        ("top", "bottom")
                    } else {
                        ("left", "right")
                    };
                    let mut margins = String::from("margin:0;");
                    if margin != "end" {
                        margins.push_str(&format!("margin-{start}:auto;"));
                    }
                    if margin != "start" {
                        margins.push_str(&format!("margin-{end}:auto;"));
                    }
                    let (width, height) = if row { (100, size) } else { (size, 100) };
                    let container = format!(
                        "display:flex;flex-direction:{direction};direction:{text_direction};width:{}px;height:{}px;align-items:flex-end",
                        if row { 200 } else { 300 },
                        if row { 300 } else { 200 }
                    );
                    let item = format!("flex:none;width:{width}px;height:{height}px;{margins}");
                    let cross = if size > 300 {
                        if !row && text_direction == "rtl" {
                            -100.0
                        } else {
                            0.0
                        }
                    } else {
                        match margin {
                            "both" => (300 - size) as f32 / 2.0,
                            "start" => (300 - size) as f32,
                            _ => 0.0,
                        }
                    };
                    let reverse = direction.ends_with("reverse") ^ (row && text_direction == "rtl");
                    let main = if reverse { 100.0 } else { 0.0 };
                    let expected = if row {
                        (main, cross, width as f32, height as f32)
                    } else {
                        (cross, main, width as f32, height as f32)
                    };
                    let actual = item_rect(&container, &item);
                    if actual != expected {
                        failures.push(format!("{direction}/{text_direction}/{margin}/{size}: {actual:?} != {expected:?}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn oversized_block_flex_and_grid_items_keep_the_cross_start_edge() {
    for display in ["block", "flex", "grid"] {
        for align in ["stretch", "flex-start"] {
            let container = format!(
                "display:flex;flex-direction:column;width:300px;height:200px;align-items:{align}"
            );
            assert_eq!(
                item_rect(
                    &container,
                    &format!("display:{display};width:400px;height:100px;margin:0 auto")
                ),
                (0.0, 0.0, 400.0, 100.0)
            );
            assert_eq!(
                item_rect(
                    &container,
                    &format!("display:{display};aspect-ratio:2/1;flex:1;margin:0 auto")
                ),
                (0.0, 0.0, 400.0, 200.0)
            );
        }
    }
}

#[test]
fn cross_margin_corrections_keep_baselines_static_positions_and_box_edges() {
    let mut failures = Vec::new();
    for &(name, markup, expected) in ADVANCED_CASES {
        let actual = border_boxes(markup);
        for &(id, rect) in expected {
            let found = actual.iter().find(|(node, _)| node == id).map(|(_, r)| *r);
            if found != Some(rect) {
                failures.push(format!("{name} #{id}: {found:?} != {rect:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const ADVANCED_CASES: &[Case] = &[
    (
        "grid-baseline",
        r#"<div style="display:grid;grid-template-columns:100px 20px;align-items:baseline;height:500px"><div id="item" style="display:flex;width:100px;height:100px"><div id="inner" style="width:20px;height:200px;margin:auto 0;flex:none"></div></div><div id="peer" style="width:20px;height:20px"></div></div>"#,
        &[
            ("item", (0.0, 0.0, 100.0, 100.0)),
            ("inner", (0.0, 0.0, 20.0, 200.0)),
            ("peer", (100.0, 180.0, 20.0, 20.0)),
        ],
    ),
    (
        "border-padding-fixed-end",
        r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;border:3px solid;padding:10px;"><div id="item" style="width:400px;height:100px;margin-left:auto;margin-right:20px"></div></div>"#,
        &[("item", (13.0, 13.0, 400.0, 100.0))],
    ),
    (
        "negative-fixed-start",
        r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id="item" style="width:400px;height:100px;margin-left:-20px;margin-right:auto"></div></div>"#,
        &[("item", (-20.0, 0.0, 400.0, 100.0))],
    ),
    (
        "negative-fixed-end",
        r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;"><div id="item" style="width:400px;height:100px;margin-left:auto;margin-right:-20px"></div></div>"#,
        &[("item", (0.0, 0.0, 400.0, 100.0))],
    ),
    (
        "relative-row",
        r#"<div style="display:flex;width:200px;height:300px;"><div id="item" style="position:relative;top:7px;width:100px;height:400px;margin: auto 0"></div></div>"#,
        &[("item", (0.0, 7.0, 100.0, 400.0))],
    ),
    (
        "absolute-ltr",
        r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;direction:ltr;"><div id="item" style="width:400px;height:100px;margin:0 auto"><div id="child" style="position:absolute;width:10px;height:10px"></div></div></div>"#,
        &[
            ("item", (0.0, 0.0, 400.0, 100.0)),
            ("child", (0.0, 0.0, 10.0, 10.0)),
        ],
    ),
    (
        "absolute-rtl",
        r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;direction:rtl;"><div id="item" style="width:400px;height:100px;margin:0 auto"><div id="child" style="position:absolute;width:10px;height:10px"></div></div></div>"#,
        &[
            ("item", (-100.0, 0.0, 400.0, 100.0)),
            ("child", (290.0, 0.0, 10.0, 10.0)),
        ],
    ),
    (
        "fixed-ltr",
        r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;direction:ltr;"><div id="item" style="width:400px;height:100px;margin:0 auto"><div id="child" style="position:fixed;width:10px;height:10px"></div></div></div>"#,
        &[
            ("item", (0.0, 0.0, 400.0, 100.0)),
            ("child", (0.0, 0.0, 10.0, 10.0)),
        ],
    ),
    (
        "fixed-rtl",
        r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;direction:rtl;"><div id="item" style="width:400px;height:100px;margin:0 auto"><div id="child" style="position:fixed;width:10px;height:10px"></div></div></div>"#,
        &[
            ("item", (-100.0, 0.0, 400.0, 100.0)),
            ("child", (290.0, 0.0, 10.0, 10.0)),
        ],
    ),
    (
        "row-baseline",
        r#"<div style="display:flex;align-items:baseline;height:500px;"><div id="item" style="display:flex;width:100px;height:100px;"><div id="inner" style="width:20px;height:200px;margin:auto 0;flex:none"></div></div><div id="peer" style="width:20px;height:20px"></div></div>"#,
        &[
            ("item", (0.0, 0.0, 100.0, 100.0)),
            ("inner", (0.0, 0.0, 20.0, 200.0)),
            ("peer", (100.0, 180.0, 20.0, 20.0)),
        ],
    ),
    (
        "row-wrap",
        r#"<div style="display:flex;width:200px;height:300px;flex-wrap:wrap;"><div id="item" style="width:100px;height:400px;margin:auto 0;flex:none"></div></div>"#,
        &[("item", (0.0, 0.0, 100.0, 400.0))],
    ),
    (
        "column-wrap",
        r#"<div style="display:flex;flex-direction:column;width:300px;height:200px;flex-wrap:wrap;"><div id="item" style="width:400px;height:100px;margin:0 auto;flex:none"></div></div>"#,
        &[("item", (0.0, 0.0, 400.0, 100.0))],
    ),
];

#[test]
fn oversized_vertical_text_with_auto_margins_starts_at_the_content_edge() {
    // WPT auto-margins-003 assumes its words fit. With Ahem its vertical
    // word is 304px in a 200px line: Chromium places auto margins at start,
    // while the reference's unsafe center alignment overflows both ends.
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let boxes = border_boxes_with_resources(
        "<div style='font:16px/16px Ahem;writing-mode:vertical-lr;display:flex;flex-direction:column;width:400px;height:200px;border:1px solid'><div id=item style='margin:auto 0'>centeredWithMargins</div></div>",
        &resources,
    );
    let rect = boxes.iter().find(|(id, _)| id == "item").unwrap().1;
    assert_eq!((rect.0, rect.1), (1.0, 1.0));
    assert!(
        (rect.2 - 16.0).abs() <= 0.02 && (rect.3 - 304.0).abs() <= 0.02,
        "{rect:?}"
    );
}
