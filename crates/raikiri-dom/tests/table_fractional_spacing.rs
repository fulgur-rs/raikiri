//! Raster table spacing follows device-pixel quantization; vector output keeps
//! fractional CSS lengths. Expected raster boxes were measured in Chromium.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");

fn page_boxes(body: &str, scale: Option<f32>) -> Vec<Vec<(String, [f32; 4])>> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{size:800px 600px;margin:0}} \
         body {{margin:8px;font:10px/10px Ahem}} div {{box-sizing:border-box}} \
         td {{padding:0}}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new()
        .fonts(fonts)
        .border_width_device_scale(scale);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("complete layout");
    };
    result
        .pages()
        .map(|page| {
            page.fragments()
                .filter_map(|fragment| {
                    let id = page.dom().attr(fragment.node(), "id")?;
                    let r = fragment.rect();
                    Some((id.to_owned(), [r.x, r.y, r.width, r.height]))
                })
                .collect()
        })
        .collect()
}

fn boxes(body: &str, scale: Option<f32>) -> Vec<(String, [f32; 4])> {
    page_boxes(body, scale).into_iter().next().expect("page")
}

#[test]
fn raster_table_spacing_matches_chromium() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("data/table-fractional-spacing.json")).expect("oracle");
    let mut mismatches = Vec::new();
    for case in data["cases"].as_array().expect("cases") {
        let scale = case["device_scale"].as_f64().expect("scale") as f32;
        let actual = boxes(case["html"].as_str().expect("markup"), Some(scale));
        for entry in case["expected"].as_array().expect("boxes") {
            let id = entry[0].as_str().expect("id");
            let (_, got) = actual.iter().find(|(name, _)| name == id).expect("box");
            let want: Vec<_> = (1..5)
                .map(|i| entry[i].as_f64().expect("coordinate") as f32)
                .collect();
            if !got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 0.04) {
                mismatches.push(format!(
                    "{}: #{id} is {got:?}, expected {want:?}",
                    case["label"]
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn vector_and_invalid_scales_preserve_fractional_spacing() {
    let body = "<table id=t style='border-spacing:0.3px 2.7px'>\
        <tr><td id=a><div style='width:20px;height:10px'>x</div></td>\
        <td><div style='width:20px;height:10px'>x</div></td></tr>\
        <tr><td><div style='width:20px;height:10px'>x</div></td>\
        <td id=d><div style='width:20px;height:10px'>x</div></td></tr></table>";
    for scale in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(f32::NAN),
        Some(f32::INFINITY),
    ] {
        let actual = boxes(body, scale);
        for (id, want) in [
            ("t", [8.0, 8.0, 40.9, 28.1]),
            ("a", [8.3, 10.7, 20.0, 10.0]),
            ("d", [28.6, 23.4, 20.0, 10.0]),
        ] {
            let (_, got) = actual.iter().find(|(name, _)| name == id).expect("box");
            assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.001));
        }
    }
}

#[test]
fn raster_spacing_reaches_repeated_headers_footers_and_row_moves() {
    let rows: String = (0..12)
        .map(|row| format!("<tr><td id=r{row}><div style='height:20px'>x</div></td></tr>"))
        .collect();
    let markup = format!(
        "<style>@page{{size:120px 100px;margin:0}}body{{margin:0}}</style>\
         <table style='border-spacing:GAP;width:80px'>\
         <thead><tr><td id=h>h</td></tr></thead>\
         <tbody>{rows}</tbody><tfoot><tr><td id=f>f</td></tr></tfoot></table>"
    );
    let raster = page_boxes(&markup.replace("GAP", "1.7px 2.7px"), Some(1.0));
    let integer = page_boxes(&markup.replace("GAP", "1px 2px"), Some(1.0));
    assert!(raster.len() >= 3);
    assert_eq!(raster, integer);
    assert!(
        raster
            .iter()
            .filter(|page| page.iter().any(|(id, _)| id == "h"))
            .count()
            >= 3
    );
    assert!(
        raster
            .iter()
            .filter(|page| page.iter().any(|(id, _)| id == "f"))
            .count()
            >= 3
    );
}

#[test]
fn a_new_raster_profile_recascades_the_original_fractional_spacing() {
    let html = "<!DOCTYPE html><style>@page{size:800px 600px;margin:0}\
        body{margin:8px;font:10px/10px Ahem}td{padding:0}</style>\
        <table id=t style='border-spacing:0.7px'><tr><td>\
        <div style='width:20px;height:10px'>x</div></td></tr></table>";
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let original = RenderResources::new().fonts(fonts.clone());
    let doc = parse_html_with_resources(html.as_bytes(), &original).expect("parse");
    for (scale, expected) in [
        (None, [21.4, 11.4]),
        (Some(1.0), [20.0, 10.0]),
        (Some(2.0), [21.0, 11.0]),
        (None, [21.4, 11.4]),
    ] {
        let resources = RenderResources::new()
            .fonts(fonts.clone())
            .border_width_device_scale(scale);
        let status = layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .expect("layout");
        let LayoutStatus::Completed(result) = status else {
            panic!("complete layout");
        };
        let page = result.pages().next().expect("page");
        let rect = page
            .fragments()
            .find(|f| page.dom().attr(f.node(), "id") == Some("t"))
            .expect("table")
            .rect();
        assert!((rect.width - expected[0]).abs() < 0.001);
        assert!((rect.height - expected[1]).abs() < 0.001);
    }
}
