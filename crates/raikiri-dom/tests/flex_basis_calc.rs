//! A mixed-unit `calc()` in `flex-basis` keeps its percentage until the flex
//! container's inner main size is known (CSS Values 4 "Computed Value").
//!
//! Fragment rects are in page coordinates, so they include the 20px page
//! margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// Border box `(x, width)` of the first fragment of `#id`.
fn inline_box(html: &str, id: &str) -> (f32, f32) {
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
    let page = result.page(0).expect("page");
    let dom = page.dom();
    let rect = page
        .fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .unwrap_or_else(|| panic!("no fragment for {id} in {html}"))
        .rect();
    (rect.x, rect.width)
}

fn assert_close(actual: f32, expected: f32, what: &str, html: &str) {
    assert!(
        (actual - expected).abs() < 0.01,
        "{html}: {what}: expected {expected}, got {actual}"
    );
}

const STYLE: &str = "@page { size: 300px 200px; margin: 20px } body { margin: 0; font: 10px/1 Ahem }\
     .flex { display: flex; width: 200px } .flex > div { flex-shrink: 0; min-width: 0; height: 10px }";

#[test]
fn mixed_calc_flex_basis_resolves_against_the_container_width() {
    for (basis, expected) in [
        ("calc(50% - 10px)", 90.0),
        ("calc(25% + 2em)", 70.0),
        // Clamped to zero at used-value time.
        ("calc(10% - 50px)", 0.0),
    ] {
        let html = format!(
            "<!DOCTYPE html><style>{STYLE}</style>\
             <div class=flex><div id=a style=\"flex-basis: {basis}\"></div><div id=b></div></div>"
        );
        let (x, width) = inline_box(&html, "a");
        assert_close(width, expected, "item width", &html);
        let (bx, _) = inline_box(&html, "b");
        assert_close(bx - x, expected, "next item offset", &html);
    }
}

#[test]
fn mixed_calc_flex_shorthand_basis_resolves_against_the_container_width() {
    let html = format!(
        "<!DOCTYPE html><style>{STYLE}</style>\
         <div class=flex><div id=a style=\"flex: 0 0 calc(100% / 3 - 20px)\"></div></div>"
    );
    let (_, width) = inline_box(&html, "a");
    assert_close(width, 200.0 / 3.0 - 20.0, "item width", &html);
}
