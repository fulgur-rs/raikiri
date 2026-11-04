//! Chromium border boxes for vertical ratio containers as grid items, with
//! an automatic width and stated height, and with multiple wrapping items.
//! Both vertical writing directions and existing no-ratio/single-item paths
//! are checked independently of page-edge clipping.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{
    IntrinsicBox, LayoutConfig, PageDefaults, ReplacedResolver, ResolveDisposition,
    ResolvedIntrinsic, ResolverError, ResolverRequest,
};

struct Portrait;

impl ReplacedResolver for Portrait {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(30.0, 120.0),
            ResolveDisposition::Ok,
        ))
    }
}

fn check(prefixes: &[&str]) {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("data/vertical-container-ratio/chromium.json"))
            .expect("recorded cases");
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .expect("fonts");
    let resources = RenderResources::new()
        .fonts(fonts)
        .replaced_resolver(&Portrait);
    let mut failures = Vec::new();
    let mut selected = 0;
    for case in fixture["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("name");
        if !prefixes.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        selected += 1;
        let body = case["body"].as_str().expect("body");
        let html = format!(
            "<!DOCTYPE html><style>@page {{ size:800px 600px;margin:0 }} \
             body {{ margin:0;font:10px/10px Ahem }} \
             body > div {{ position:absolute;left:200px;top:150px }}</style>{body}"
        );
        let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
        let LayoutStatus::Completed(result) = layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(&resources),
        )
        .expect("layout") else {
            panic!("incomplete layout: {name}")
        };
        let page = result.pages().next().expect("page");
        for (id, key) in [("i", "rect"), ("p", "parent_rect")] {
            if key == "parent_rect" && case[key].is_null() {
                continue;
            }
            let fragment = page
                .fragments()
                .find(|f| page.dom().attr(f.node(), "id") == Some(id))
                .unwrap_or_else(|| panic!("missing #{id}: {name}"));
            let rect = fragment.rect();
            let actual = [rect.x - 200.0, rect.y - 150.0, rect.width, rect.height];
            let expected: [f32; 4] =
                std::array::from_fn(|i| case[key][i].as_f64().expect("coordinate") as f32);
            if actual
                .iter()
                .zip(expected)
                .any(|(a, b)| (a - b).abs() > 0.02)
            {
                failures.push(format!("{name} #{id}: {actual:?}, expected {expected:?}"));
            }
        }
    }
    assert!(selected > 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_vertical_grid_item_can_raise_its_ratio_height_to_fit_content() {
    check(&["grid_item_"]);
}

#[test]
fn a_vertical_container_with_a_stated_height_derives_its_auto_width_from_ratio() {
    check(&["auto_width_"]);
}

#[test]
fn multiple_wrapping_items_raise_the_vertical_ratio_height_to_the_largest_item() {
    check(&["wrapped_items_"]);
}

#[test]
fn existing_no_ratio_and_single_item_sizing_stays_unchanged() {
    check(&["no_ratio_", "single_item_"]);
}
