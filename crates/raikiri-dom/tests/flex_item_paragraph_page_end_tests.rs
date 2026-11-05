//! A paragraph in a flex container that holds nothing else moves its lines at
//! a page end as it does in plain block flow: the container grows around
//! them and the flow after it follows.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page, word and glyph top of each paragraph word `z` and marker `C`,
/// `D` or `E`, in page order.
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
            for word in run.text.split_whitespace() {
                if matches!(word, "z" | "C" | "D" | "E") {
                    glyphs.push((page.index(), word.to_owned(), run.origin.1 - run.ascent));
                }
            }
        }
    }
    glyphs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2)));
    glyphs
}

#[test]
fn paragraphs_in_flex_containers_with_one_item_match_printed_lines_and_following_boxes() {
    // Chromium PDF glyph bounds with loaded Ahem, 100px pages and CSS page
    // sizing. The paragraph is four or eight 30px lines `z` in a 20px column;
    // `C` follows the flex container inside its wrapper and `D` follows the
    // wrapper. The cases cover narrow and wrapping containers, `row-reverse`
    // and centered items, padding, a container with a `min-height` or
    // `height`, flex containers nested in flex containers and in blocks, a
    // paragraph or several in a block inside the item, items and paragraphs
    // with a height of their own, column containers, and orphans or widows
    // that move the whole paragraph or pull lines back from the page end.
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("data/flex-item-paragraph-page-end.json")).unwrap();
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
