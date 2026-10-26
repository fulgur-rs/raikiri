//! Layout recursion: the deepest tree layout accepts is laid out for every kind
//! of box, so the limit on nested layouts (which stops a layout that never
//! ends) is out of the way of every document.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use url::Url;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The `<div>`s that fit under `<body>` in a tree of the deepest accepted
/// depth, which counts `<body>` itself.
const DIVS: usize = raikiri_dom::MAX_LAYOUT_DEPTH - 1;

/// The number of pages `html` lays out to, or the error it ends with.
fn pages(html: &str) -> Result<u32, String> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 400px 600px; margin: 20px }} \
         body {{ margin: 0; font: 10px/1 Ahem }}</style><body>{html}</body>"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://recursion.test/document.html").expect("url"));
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    match layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    ) {
        Ok(LayoutStatus::Completed(result)) => Ok(result.page_count()),
        Ok(_) => Err("the layout did not complete".to_owned()),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// `levels` boxes with the style of each entry of `styles` in turn, each in the
/// one before, around some text.
fn nested(styles: &[&str], levels: usize) -> String {
    let mut html = String::new();
    for level in 0..levels {
        html.push_str(&format!("<div style=\"{}\">", styles[level % styles.len()]));
    }
    html.push_str("text in the middle");
    html.push_str(&"</div>".repeat(levels));
    html
}

#[test]
fn a_tree_of_the_deepest_accepted_depth_is_laid_out_for_every_kind_of_box() {
    for style in [
        "display:block",
        "display:flex",
        "display:flex;flex-direction:column",
        "display:grid",
        "display:flow-root",
        "display:table-cell",
        "float:left",
        "position:relative",
        "contain:size;width:100px;height:100px",
        "overflow:hidden",
        "columns:2",
    ] {
        let result = pages(&nested(&[style], DIVS));
        assert!(result.is_ok(), "{style}: {result:?}");
    }
}

#[test]
fn a_tree_of_the_deepest_accepted_depth_is_laid_out_for_kinds_of_boxes_in_turn() {
    let result = pages(&nested(
        &[
            "display:block",
            "display:flex",
            "display:grid",
            "display:flow-root",
            "display:flex;flex-direction:column",
            "display:table-cell",
            "float:left",
            "columns:2",
        ],
        DIVS,
    ));
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn a_tree_deeper_than_accepted_is_refused_before_any_layout() {
    let result = pages(&nested(&["display:block"], DIVS + 1));
    let error = result.expect_err("a tree one element too deep");
    assert!(error.contains("TreeDepthLimitExceeded"), "{error}");
}
