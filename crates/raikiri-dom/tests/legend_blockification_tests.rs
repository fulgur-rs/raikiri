//! A `legend` is blockified wherever it stands, as Chromium does: an
//! inline-level legend lays out as its block-level equivalent.

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
fn an_inline_level_legend_lays_out_as_a_block_level_box() {
    // Chromium `getBoundingClientRect` with loaded Ahem at 10px on an 800px
    // page; `o` is a 200px wrapper, `c` a fieldset, `l` the legend. The
    // cases cover `inline`, `inline-block`, `inline-flex`, `inline-grid`,
    // `inline-table`, `table-cell`, `table-row` and `list-item` legends, as
    // the rendered legend of a fieldset of block content and outside any
    // fieldset, with a stated width, margins, a block child, a child margin
    // that collapses through the legend and a float that it does not hold.
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("data/legend-blockification.json")).unwrap();
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
