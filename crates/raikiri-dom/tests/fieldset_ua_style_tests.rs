//! The user agent style of a `<fieldset>`: a 2px groove border, padding in
//! `em`, a 2px inline margin and a `min-inline-size` of `min-content`.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border box `[x, y, width, height]` of the first fragment of every
/// element with an `id` on the first page.
fn rects(html: &str) -> std::collections::BTreeMap<String, [f32; 4]> {
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
    let page = result.pages().next().unwrap();
    let dom = page.dom();
    let mut rects = std::collections::BTreeMap::new();
    for fragment in page.fragments() {
        let Some(id) = dom.attr(fragment.node(), "id") else {
            continue;
        };
        let rect = fragment.rect();
        rects
            .entry(id.to_owned())
            .or_insert([rect.x, rect.y, rect.width, rect.height]);
    }
    rects
}

#[test]
fn a_fieldset_has_the_border_padding_margin_and_minimum_size_of_the_user_agent_style() {
    // Chromium `getBoundingClientRect` with loaded Ahem at 10px on an
    // 800px page; `o` is a 200px wrapper and `c` the fieldset (`d` and `e`
    // its descendants or a sibling). The cases cover an empty, sized and
    // border-box fieldset, `min-content` as the smallest inline size and its
    // reset to `0`, overrides of the margin, border and padding, another font
    // size, vertical writing modes, `rtl`, auto margins, floats, inline
    // blocks, flex items, absolute positioning, nested fieldsets, table
    // cells, flex and grid fieldsets and a percentage height.
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("data/fieldset-ua-style.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let expected: std::collections::BTreeMap<String, [f32; 4]> =
            serde_json::from_value(case["rects"].clone()).unwrap();
        let actual = rects(case["html"].as_str().unwrap());
        for (id, expected) in &expected {
            let actual = actual
                .get(id)
                .unwrap_or_else(|| panic!("{name}: no fragment for #{id}"));
            for (actual, expected) in actual.iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 0.02,
                    "{name}: #{id} {actual:?} != {expected:?}"
                );
            }
        }
    }
}
