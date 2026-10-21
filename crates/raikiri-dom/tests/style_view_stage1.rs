//! Production-layout regressions for the stage-one Flex/Grid style adapter.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

fn boxes(html: &str, ids: &[&str]) -> Vec<(f32, f32, f32, f32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("expected complete layout")
    };
    let page = result.page(0).unwrap();
    let dom = page.dom();
    ids.iter()
        .map(|id| {
            let rect = page
                .fragments()
                .find(|fragment| dom.attr(fragment.node(), "id") == Some(*id))
                .unwrap()
                .rect();
            (rect.x, rect.y, rect.width, rect.height)
        })
        .collect()
}

fn assert_box(actual: (f32, f32, f32, f32), expected: (f32, f32, f32, f32)) {
    for (actual, expected) in [actual.0, actual.1, actual.2, actual.3]
        .into_iter()
        .zip([expected.0, expected.1, expected.2, expected.3])
    {
        assert!(
            (actual - expected).abs() < 0.01,
            "expected {expected}, got {actual}"
        );
    }
}

const BASE: &str = "@page{size:300px 300px;margin:20px}body{margin:0;font:10px/1 Ahem}";

#[test]
fn flex_calc_natural_svg_and_anonymous_text_keep_alignment() {
    let html = format!(
        "<!doctype html><style>{BASE}.flex{{display:flex;width:200px;height:40px;gap:5px;align-items:center}}</style><div class=flex><svg id=natural xmlns='http://www.w3.org/2000/svg' width=40 height=20><rect width=40 height=20 /></svg><div id=calc style='flex:0 0 calc(50% - 10px);height:10px'></div>xx<div id=tail style='flex:none;width:10px;height:10px'></div></div>"
    );
    let rects = boxes(&html, &["natural", "calc", "tail"]);
    assert_box(rects[0], (20.0, 30.0, 40.0, 20.0));
    assert_box(rects[1], (65.0, 35.0, 90.0, 10.0));
    assert_box(rects[2], (185.0, 35.0, 10.0, 10.0));
}

#[test]
fn grid_calc_repeats_and_contents_items_keep_self_alignment() {
    let html = format!(
        "<!doctype html><style>{BASE}.grid{{display:grid;width:200px;grid-template-columns:repeat(2,calc(25% + 10px));grid-auto-rows:20px;gap:5px;justify-items:end;align-items:center}}.item{{width:30px;height:10px}}</style><div class=grid><div id=a class=item></div><div id=b class=item></div><div style='display:contents'><div id=c class=item></div></div></div>"
    );
    let rects = boxes(&html, &["a", "b", "c"]);
    assert_box(rects[0], (50.0, 25.0, 30.0, 10.0));
    assert_box(rects[1], (115.0, 25.0, 30.0, 10.0));
    assert_box(rects[2], (50.0, 50.0, 30.0, 10.0));
}

#[test]
fn grid_query_keeps_position_based_claims_for_static_position() {
    for policy in ["transform:translateX(0)", "contain:layout size"] {
        let html = format!(
            "<!doctype html><style>{BASE}.grid{{display:grid;width:200px;height:40px;grid-template-columns:100px 100px;grid-template-rows:40px;{policy}}}.abs{{position:absolute;grid-column:2;grid-row:1;width:10px;height:10px}}</style><div class=grid><div id=abs class=abs></div></div>"
        );
        assert_box(boxes(&html, &["abs"])[0], (20.0, 20.0, 10.0, 10.0));
        let html = html.replace("display:grid", "display:grid;position:relative");
        assert_box(boxes(&html, &["abs"])[0], (120.0, 20.0, 10.0, 10.0));
    }
}
