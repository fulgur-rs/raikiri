#![allow(dead_code)]
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::HashMap;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border box `(x, y, width, height)` of every element with an `id` on the
/// first page of an 800x600 document that holds `body`.
fn boxes(body: &str) -> HashMap<String, [f32; 4]> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
    );
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let mut found = HashMap::new();
    for fragment in page.fragments() {
        if let Some(id) = dom.attr(fragment.node(), "id") {
            let rect = fragment.rect();
            found.insert(id.to_string(), [rect.x, rect.y, rect.width, rect.height]);
        }
    }
    found
}

#[test]
fn scratch_probe() {
    let cases: Vec<String> = serde_json::from_str(&std::fs::read_to_string("/tmp/claude-0/-home-user/03da35c2-c3bc-58cb-a760-ce0389db8275/scratchpad/p1628/set4/cases.json").unwrap()).unwrap();
    let out: Vec<_> = cases.iter().map(|c| boxes(c)).collect();
    std::fs::write("/tmp/claude-0/-home-user/03da35c2-c3bc-58cb-a760-ce0389db8275/scratchpad/p1628/set4/raikiri.json", serde_json::to_string(&out).unwrap()).unwrap();
}
