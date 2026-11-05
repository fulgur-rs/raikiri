//! A paragraph that crosses a page end moves its lines inside ancestors whose
//! heights are constrained, as it does inside ancestors that follow their
//! content.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page, text and glyph top of each paragraph word `X` and marker `C` or
/// `D`, in page order.
fn glyphs(html: &str) -> Vec<(u32, String, f32)> {
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
                glyphs.push((page.index(), text.to_owned(), run.origin.1 - run.ascent));
            }
        }
    }
    glyphs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2)));
    glyphs
}

#[test]
fn paragraphs_in_height_constrained_wrappers_match_printed_lines_and_following_boxes() {
    // Chromium PDF glyph bounds with loaded Ahem, 100px pages and CSS page
    // sizing. The paragraph is four or five 30px lines `X`, `C` follows it
    // inside its wrapper and `D` follows the wrapper: a wrapper keeps its set
    // height, so `D` stays at the wrapper's end however far the lines move,
    // while `C` follows the moved lines. The cases cover `height`,
    // `min-height` and `max-height` wrappers, nested wrappers, padding,
    // formatting contexts, list items, a paragraph with a height of its own,
    // and a paragraph that moves as a whole with the wrapper it begins.
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("data/constrained-ancestor-paragraph.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut expected: Vec<(u32, String, f32)> =
            serde_json::from_value(case["glyphs"].clone()).unwrap();
        expected.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2)));
        let actual = glyphs(case["html"].as_str().unwrap());
        assert_eq!(actual.len(), expected.len(), "{name}: {actual:?}");
        for (actual, expected) in actual.iter().zip(expected.iter()) {
            assert_eq!((&actual.0, &actual.1), (&expected.0, &expected.1), "{name}");
            assert!(
                (actual.2 - expected.2).abs() < 0.001,
                "{name}: {actual:?} != {expected:?}"
            );
        }
    }
}
