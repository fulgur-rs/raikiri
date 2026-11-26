//! The boxes inside an inline-block after the paragraph around it was measured.
//!
//! A paragraph measures its min-content and max-content widths by laying out
//! its inline-blocks at each width it tries, and that layout replaces what the
//! boxes inside them held. When the paragraph is measured again after its
//! final layout, as a column flex container does with its items, the layout
//! that follows must lay those boxes out again at the paragraph's width rather
//! than keep them as the widest measure left them. The expected border boxes
//! are the ones Chromium lays out for the same markup, in an 800px by 600px
//! viewport without a body margin, with Ahem at 10px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page without margins.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
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
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    let laid_out = boxes(body);
    for (id, rect) in expected {
        let found = laid_out
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no box for #{id} in {laid_out:?}"))
            .1;
        assert!(
            found.iter().zip(rect).all(|(a, b)| (a - b).abs() < 0.01),
            "#{id}: expected {rect:?}, got {found:?} (all: {laid_out:?})"
        );
    }
}

/// Two inline-blocks inside an item of a column flex container that a
/// min-content box measures: the text inside wraps at the item's 30px.
#[test]
fn nested_inline_blocks_in_a_measured_column_item_keep_the_item_width() {
    assert_boxes(
        r#"<div style="width:min-content"><div style="display:flex;flex-direction:column"><div style="max-width:30px"><span id=o style="display:inline-block"><span id=i style="display:inline-block"><div id=t>AA BB CCC</div></span></span></div></div></div>"#,
        &[
            ("o", [0.0, 0.0, 30.0, 30.0]),
            ("i", [0.0, 0.0, 30.0, 30.0]),
            ("t", [0.0, 0.0, 30.0, 30.0]),
        ],
    );
}

/// The same item inside a float, which measures it to shrink to fit.
#[test]
fn nested_inline_blocks_in_a_column_item_of_a_float_keep_the_item_width() {
    assert_boxes(
        r#"<div style="float:left"><div style="display:flex;flex-direction:column"><div style="max-width:30px"><span id=o style="display:inline-block"><span id=i style="display:inline-block"><div id=t>AA BB CCC</div></span></span></div></div></div>"#,
        &[
            ("o", [0.0, 0.0, 30.0, 30.0]),
            ("i", [0.0, 0.0, 30.0, 30.0]),
            ("t", [0.0, 0.0, 30.0, 30.0]),
        ],
    );
}

/// A column flex container inside the inner inline-block, with text around
/// both inline-blocks: its items wrap at 30px too.
#[test]
fn a_column_flex_container_in_nested_inline_blocks_keeps_the_item_width() {
    assert_boxes(
        r#"<div style="width:max-content"><div style="display:flex;flex-direction:column"><div style="max-width:30px">X<span id=o style="display:inline-block">X<span id=i style="display:inline-block"><div id=t style="display:flex;flex-direction:column"><div id=a>AA BB CCC</div><div id=b>DD EE</div></div></span></span>X</div></div></div>"#,
        &[
            ("o", [0.0, 10.0, 30.0, 60.0]),
            ("i", [0.0, 20.0, 30.0, 50.0]),
            ("t", [0.0, 20.0, 30.0, 50.0]),
            ("a", [0.0, 20.0, 30.0, 30.0]),
            ("b", [0.0, 50.0, 30.0, 20.0]),
        ],
    );
}
