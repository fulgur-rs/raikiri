//! Outside text markers use the full advance of their widest line.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");
const CSS: &str = include_str!("data/outside-marker-offset.css");

fn sort_glyphs(glyphs: &mut [(String, f32)]) {
    glyphs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
}

#[test]
fn text_marker_horizontal_positions_match_chromium_print() {
    // The PDF reference uses the same Ahem face. Each nonblank character has
    // an advance equal to its font size; raw PDF word boxes remain in the data.
    // Only horizontal positions are compared: this regression does not change
    // the independent marker baseline or vertical-writing contracts.
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("data/outside-marker-offset.json")).unwrap();
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let mut mismatches = Vec::new();
    for case in oracle["cases"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let html = format!(
            "<!doctype html><style>{CSS}{}</style>{}",
            case["extra_css"].as_str().unwrap(),
            case["body"].as_str().unwrap(),
        );
        let doc = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
        let LayoutStatus::Completed(result) = layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .unwrap() else {
            panic!("{label}: incomplete layout")
        };
        let mut actual = Vec::new();
        for page in result.pages() {
            for run in page.text_runs() {
                for (index, ch) in run.text.chars().enumerate() {
                    if ch.is_whitespace() {
                        continue;
                    }
                    let origin = (run.origin.0 + index as f32 * run.font_size, run.origin.1);
                    let x = run.transform.map_or(origin, |t| t.apply(origin)).0;
                    actual.push((ch.to_string(), x));
                }
            }
        }
        let mut expected: Vec<(String, f32)> =
            serde_json::from_value(case["glyph_x"].clone()).unwrap();
        sort_glyphs(&mut actual);
        sort_glyphs(&mut expected);
        if result.pages().count() != case["pages"].as_array().unwrap().len()
            || actual.len() != expected.len()
            || actual
                .iter()
                .zip(&expected)
                .any(|(got, want)| got.0 != want.0 || (got.1 - want.1).abs() > 0.05)
        {
            mismatches.push(format!("{label}: {actual:?}, expected {expected:?}"));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
