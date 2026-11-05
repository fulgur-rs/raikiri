//! Page placements for abspos boxes before the first page (RAIKIRI-1178).
//!
//! Chromium print-media DOMRects still report the original zero-size box
//! at (-150, -150), but its PDF has no ink there. Page fragments describe
//! placements, not every DOM box. The fixture records print settings,
//! measured DOMRects and PDF raster controls; it makes no claim about
//! Chromium's internal fragments. A partially visible box remains present,
//! as do an on-page zero-size box and a box outside only the horizontal edge.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");

#[test]
fn abspos_placements_respect_the_first_page_block_extent() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("data/negative-abspos-print.json")).expect("fixture");
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    for case in fixture["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("name");
        let margin = case["margin"].as_u64().expect("margin");
        let body = case["body"].as_str().expect("body");
        let html = format!(
            "<!doctype html><style>@page {{ size: 240px 240px; margin: 0 }} \
             body {{ margin: {margin}px }} * {{ print-color-adjust: exact }}</style>{body}"
        );
        let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
        let LayoutStatus::Completed(result) = layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .expect("layout") else {
            panic!("expected complete layout for {name}");
        };
        let pages = result.pages().collect::<Vec<_>>();
        assert_eq!(pages.len(), 1, "{name}: page count");
        let page = &pages[0];
        let actual = page
            .fragments()
            .filter(|fragment| page.dom().attr(fragment.node(), "id") == Some("t"))
            .map(|fragment| {
                let rect = fragment.rect();
                [rect.x, rect.y, rect.width, rect.height]
            })
            .collect::<Vec<_>>();
        let expected = case["page_placements"].as_array().expect("placements");
        assert_eq!(actual.len(), expected.len(), "{name}: {actual:?}");
        for (actual, expected) in actual.iter().zip(expected) {
            let expected = expected.as_array().expect("rect");
            assert_eq!(expected.len(), 4);
            for (value, expected) in actual.iter().zip(expected) {
                let expected = expected.as_f64().expect("coordinate") as f32;
                assert!(
                    (value - expected).abs() <= 0.02,
                    "{name}: {actual:?}, expected {expected}"
                );
            }
        }
    }
}
