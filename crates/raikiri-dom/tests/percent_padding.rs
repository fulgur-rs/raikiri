//! Percentage padding resolves against the containing block's width on all
//! four sides (CSS Box 4 §4: "Percentages: refer to logical width of
//! containing block"), including `padding-top` and `padding-bottom`.
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

/// Border box `(x, y, width, height)` and content box `(x, y, width, height)`
/// of the first fragment of `#id`.
type Boxes = ((f32, f32, f32, f32), (f32, f32, f32, f32));

fn boxes(html: &str, id: &str) -> Boxes {
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
    let fragment = page
        .fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .unwrap_or_else(|| panic!("no fragment for {id} in {html}"));
    let rect = fragment.rect();
    let content = fragment.content_rect().expect("content rect");
    (
        (rect.x, rect.y, rect.width, rect.height),
        (content.x, content.y, content.width, content.height),
    )
}

fn assert_close(actual: f32, expected: f32, what: &str, html: &str) {
    assert!(
        (actual - expected).abs() < 0.01,
        "{html}: {what}: expected {expected}, got {actual}"
    );
}

/// Page area 260px x 160px, so 10% is 26px of the width and would be 16px of
/// the height.
const PAGE: &str = "@page { size: 300px 200px; margin: 20px } body { margin: 0 }";

#[test]
fn vertical_percent_padding_of_a_block_uses_the_containing_block_width() {
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} p {{ margin: 0; height: 30px }}</style>\
         <p id=p style=\"border: 5px solid transparent; padding: 10%\"></p>"
    );
    let ((x, y, width, height), (cx, cy, cw, ch)) = boxes(&html, "p");
    assert_close(width, 260.0, "border-box width", &html);
    assert_close(height, 30.0 + 10.0 + 52.0, "border-box height", &html);
    assert_close(cx - x, 5.0 + 26.0, "content left inset", &html);
    assert_close(cy - y, 5.0 + 26.0, "content top inset", &html);
    assert_close(cw, 260.0 - 10.0 - 52.0, "content width", &html);
    assert_close(ch, 30.0, "content height", &html);
}

#[test]
fn vertical_percent_padding_with_border_box_sizing() {
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} p {{ margin: 0; height: 100px; box-sizing: border-box }}</style>\
         <p id=p style=\"padding: 10% 0\"></p>"
    );
    let ((_, y, _, height), (_, cy, _, ch)) = boxes(&html, "p");
    assert_close(height, 100.0, "border-box height", &html);
    assert_close(cy - y, 26.0, "content top inset", &html);
    assert_close(ch, 100.0 - 52.0, "content height", &html);
}

#[test]
fn vertical_percent_padding_of_a_nested_block_uses_its_parent_width() {
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} div {{ width: 200px; padding: 0 30px }}\
         p {{ margin: 0; height: 10px }}</style>\
         <div><p id=p style=\"padding: 10% 0\"></p></div>"
    );
    let ((_, y, _, height), (_, cy, _, _)) = boxes(&html, "p");
    assert_close(height, 10.0 + 40.0, "border-box height", &html);
    assert_close(cy - y, 20.0, "content top inset", &html);
}

#[test]
fn vertical_percent_padding_of_a_block_after_a_sibling() {
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} div {{ height: 50px }} p {{ margin: 0; height: 10px }}</style>\
         <div></div><p id=p style=\"padding-bottom: 10%\"></p><div id=after></div>"
    );
    let ((_, y, _, height), _) = boxes(&html, "p");
    assert_close(y, 20.0 + 50.0, "top", &html);
    assert_close(height, 10.0 + 26.0, "border-box height", &html);
    let ((_, after_y, _, _), _) = boxes(&html, "after");
    assert_close(after_y, 20.0 + 50.0 + 36.0, "next sibling top", &html);
}

#[test]
fn vertical_percent_padding_of_an_abspos_box_uses_its_containing_block_width() {
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} div {{ position: relative; width: 200px; height: 50px; padding: 0 30px }}\
         p {{ position: absolute; margin: 0; height: 10px }}</style>\
         <div><p id=p style=\"padding: 10% 0\"></p></div>"
    );
    // The containing block is the padding box of the div: 260px wide.
    let ((_, y, _, height), (_, cy, _, _)) = boxes(&html, "p");
    assert_close(height, 10.0 + 52.0, "border-box height", &html);
    assert_close(cy - y, 26.0, "content top inset", &html);
}

#[test]
fn vertical_percent_padding_inside_an_auto_width_float_uses_its_derived_width() {
    // The float is 100px wide from its content but 50px tall; 10% is 10px.
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} div {{ float: left; height: 50px }}\
         p {{ margin: 0; width: 100px; height: 10px }}</style>\
         <div><p id=p style=\"padding: 10% 0\"></p></div>"
    );
    let ((_, y, _, height), (_, cy, _, _)) = boxes(&html, "p");
    assert_close(cy - y, 10.0, "content top inset", &html);
    assert_close(height, 10.0 + 20.0, "border-box height", &html);
}
