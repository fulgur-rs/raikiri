//! Height-constrained paragraphs retain complete lines inside block wrappers.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

fn glyphs(html: &str) -> (Vec<(u32, String, f32)>, f32, f32) {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    let mut glyphs = Vec::new();
    for page in result.pages() {
        for run in page.text_runs() {
            let text = run.text.trim();
            if matches!(text, "X" | "C" | "D") {
                let origin = (run.origin.0, run.origin.1 - run.ascent);
                let top = run
                    .transform
                    .map_or(origin, |transform| transform.apply(origin))
                    .1;
                glyphs.push((page.index(), text.to_owned(), top));
            }
        }
    }
    glyphs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2)));
    let (document, _, _, _) = result.page(0).unwrap().paint_inputs();
    let paragraph = (0..document.node_count())
        .find(|id| document.get_node(*id).unwrap().attribute("id") == Some("paragraph"))
        .unwrap();
    (
        glyphs,
        document.get_node(paragraph).unwrap().layout().size.height,
        document.layout_origin_of(paragraph).unwrap().y,
    )
}

#[test]
fn constrained_nested_paragraphs_match_printed_lines_and_following_boxes() {
    // Chromium PDF glyph bounds with loaded Ahem, a 100px viewport, and CSS
    // page sizing. C follows the paragraph inside its wrapper; D follows the
    // wrapper. These markers detect incorrect propagation of line growth.
    // The cases cover fixed/min/max heights, partial growth, orphan and widow
    // moves, multiple inline owners, and clipped paragraphs. For clipped
    // paragraphs compare the box and following markers: text_runs exposes
    // layout geometry, whereas printed text also reflects paint clipping.
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("data/nested-height-paragraph.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let mut expected: Vec<(u32, String, f32)> =
            serde_json::from_value(case["glyphs"].clone()).unwrap();
        expected.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2)));
        let (mut actual, height, top) = glyphs(case["html"].as_str().unwrap());
        if let Some(texts) = case["compare_text"].as_array() {
            actual.retain(|(_, text, _)| texts.iter().any(|wanted| wanted.as_str() == Some(text)));
        }
        assert_eq!(
            top,
            case["paragraph_top"].as_f64().unwrap() as f32,
            "{}",
            case["name"]
        );
        assert_eq!(
            height,
            case["paragraph_height"].as_f64().unwrap() as f32,
            "{}",
            case["name"]
        );
        assert_eq!(actual.len(), expected.len(), "{}: {actual:?}", case["name"]);
        for (actual, expected) in actual.iter().zip(expected.iter()) {
            assert_eq!(
                (&actual.0, &actual.1),
                (&expected.0, &expected.1),
                "{}",
                case["name"]
            );
            assert!(
                (actual.2 - expected.2).abs() < 0.001,
                "{}: {actual:?} != {expected:?}",
                case["name"]
            );
        }
    }
}
