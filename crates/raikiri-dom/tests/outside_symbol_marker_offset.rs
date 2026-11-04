//! Outside `disc`, `circle` and `square` markers start a font-derived gutter
//! before the principal box, as Chromium's vector bullets do.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");
const NOTO: &[u8] = include_bytes!("data/noto-sans-test/NotoSansTest-Regular.ttf");

/// The x of the first glyph of the marker of the only list item, with the item
/// content starting at x = 100 and the text in Ahem.
fn marker_x(body_font_size: u32, extra_css: &str) -> f32 {
    marker_x_in("Ahem", body_font_size, extra_css)
}

/// [`marker_x`] with the text in `family`, one of the fonts the fixture loads.
fn marker_x_in(family: &str, body_font_size: u32, extra_css: &str) -> f32 {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .font_bytes("NotoTest", NOTO.to_vec())
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:400px 200px;margin:0}}\
         html,body{{margin:0}}body{{font:{body_font_size}px/1.4 {family}}}\
         ul{{margin:0;padding-left:100px}}{extra_css}</style><ul><li>X</li></ul>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let runs = result.page(0).unwrap().text_runs();
    let marker = runs
        .iter()
        .find(|run| run.is_standalone_marker() && !run.text.trim().is_empty())
        .expect("the item has a marker");
    marker.origin.0
}

#[test]
fn symbol_bullets_start_at_the_chromium_gutter() {
    // Chromium 153.0.8010.12 headless screenshots at 4x, bundled Ahem. The
    // left edge of the painted bullet, from the item content edge, is
    // `ascent * 2 / 3 + 7` with the ascent rounded to whole pixels
    // (0.8em for Ahem), for all three bullet keywords.
    let oracle = [
        (10, 12.0),
        (12, 13.0),
        (16, 15.0),
        (20, 17.0),
        (24, 19.0),
        (30, 23.0),
        (40, 28.0),
    ];
    let mut mismatches = Vec::new();
    for keyword in ["disc", "circle", "square"] {
        for (size, gutter) in oracle {
            let x = marker_x(size, &format!("ul{{list-style:{keyword}}}"));
            if (x - (100.0 - gutter)).abs() > 0.05 {
                mismatches.push(format!(
                    "{keyword} at {size}px: x {x}, expected {}",
                    100.0 - gutter
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_proportional_font_gets_the_gutter_of_its_own_ascent() {
    // Chromium 153.0.8010.12 with the same face: its ascent is not 0.8em, and
    // rounding it to whole pixels first moves the gutter between sizes.
    let oracle = [
        (10, 13.0),
        (12, 15.0),
        (13, 15.0),
        (14, 16.0),
        (16, 17.0),
        (20, 20.0),
        (24, 23.0),
        (30, 27.0),
        (40, 33.0),
    ];
    for (size, gutter) in oracle {
        assert_eq!(
            marker_x_in("NotoTest", size, ""),
            100.0 - gutter,
            "a disc at {size}px"
        );
    }
}

#[test]
fn the_default_marker_is_a_disc() {
    assert_eq!(marker_x(16, ""), 85.0);
}

#[test]
fn the_marker_font_sets_the_gutter() {
    // The item at 32px with a 16px `::marker` gets the 16px gutter, and the
    // reverse gets the 32px one (ascent 26, gutter 24).
    assert_eq!(
        marker_x(16, "li{font-size:32px}li::marker{font-size:16px}"),
        85.0
    );
    assert_eq!(marker_x(16, "li::marker{font-size:32px}"), 76.0);
}

#[test]
fn a_styled_marker_keeps_the_bullet_gutter() {
    assert_eq!(marker_x(16, "li::marker{color:red}"), 85.0);
}

#[test]
fn text_markers_keep_the_full_advance() {
    // A marker the author gives as text, through `content` or a string
    // `list-style-type`, is laid out as text: Chromium ends it at the content
    // edge. Ahem's "M " advances 32px at 16px, and "1. " 48px.
    assert_eq!(marker_x(16, "li::marker{content:\"M \"}"), 68.0);
    assert_eq!(marker_x(16, "ul{list-style-type:\"M \"}"), 68.0);
    assert_eq!(marker_x(16, "ul{list-style-type:decimal}"), 52.0);
}
