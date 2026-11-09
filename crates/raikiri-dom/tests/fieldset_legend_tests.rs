//! The rendered legend of a `<fieldset>`: it sits over the block-start
//! border, and the content of the fieldset starts below it.

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
fn the_rendered_legend_sits_over_the_block_start_border_of_its_fieldset() {
    // Chromium `getBoundingClientRect` with loaded Ahem at 10px on an 800px
    // page; `o` is a 200px wrapper, `c` the fieldset, `l` its rendered
    // legend and `t` the block after it. The cases cover a legend taller and
    // shorter than the border, its margins and padding, auto margins, `rtl`,
    // a stated and a content width, a fieldset with a stated height, an
    // inline-block and a flex item fieldset, legends that are not rendered (a
    // second, floated, absolutely positioned or hidden one), a legend after
    // the content, a relatively positioned legend, a float in the content,
    // the box after the fieldset, a percentage height below a legend that
    // reaches past the border, `min-content` and `max-content` legend widths
    // and an absolutely positioned box in the legend.
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("data/fieldset-legend.json")).unwrap();
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
