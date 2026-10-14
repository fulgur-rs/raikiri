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

#[test]
fn calc_vertical_padding_inside_an_auto_width_float_uses_its_derived_width() {
    // At the 1px probe basis `calc(100% - 20px)` clamps to zero, but against
    // the float's derived 100px width it is 80px.
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} div {{ float: left; height: 50px }}\
         p {{ margin: 0; width: 100px; height: 10px }}</style>\
         <div><p id=p style=\"padding: calc(100% - 20px) 0 0\"></p></div>"
    );
    let ((_, y, _, _), (_, cy, _, _)) = boxes(&html, "p");
    assert_close(cy - y, 80.0, "content top inset", &html);
}

#[test]
fn calc_padding_on_a_table_resolves_against_its_containing_block() {
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} div {{ width: 200px }}\
         table {{ border-spacing: 0; padding-left: calc(10% + 2px) }}\
         td {{ padding: 0; width: 10px; height: 10px }}</style>\
         <div><table id=t><tr><td id=c></td></tr></table></div>"
    );
    let ((x, _, _, _), _) = boxes(&html, "t");
    let ((cx, _, _, _), _) = boxes(&html, "c");
    assert_close(cx - x, 22.0, "table content left inset", &html);
}

#[test]
fn calc_padding_of_a_flex_item_resolves_against_the_flex_container_width() {
    // The automatic minimum size of the item adds its padding to its
    // min-content size. Against the 200px container `calc(100px - 50%)` is
    // zero, so the item keeps its 100px flex base size.
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} body {{ font: 20px/1 Ahem }}\
         .flex {{ display: flex; width: 200px }}</style>\
         <div class=flex><div id=i style=\"flex-basis: 50%; padding-right: calc(100px - 50%)\">XX</div></div>"
    );
    let ((_, _, width, _), (_, _, cw, _)) = boxes(&html, "i");
    assert_close(width, 100.0, "border-box width", &html);
    assert_close(cw, 100.0, "content width", &html);
}

#[test]
fn percent_padding_of_a_flex_item_counts_in_its_automatic_minimum_size() {
    // 10% of the 200px container is 20px, added to the 40px min-content
    // size of the text.
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} body {{ font: 20px/1 Ahem }}\
         .flex {{ display: flex; width: 200px }}</style>\
         <div class=flex><div id=i style=\"flex-basis: 0; padding-right: 10%\">XX</div></div>"
    );
    let ((_, _, width, _), (_, _, cw, _)) = boxes(&html, "i");
    assert_close(width, 60.0, "border-box width", &html);
    assert_close(cw, 40.0, "content width", &html);
}

#[test]
fn percent_margin_of_a_flex_item_stays_outside_its_automatic_minimum_size() {
    // The margin moves the next item but leaves the border box at the
    // 40px min-content size of the text.
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} body {{ font: 20px/1 Ahem }}\
         .flex {{ display: flex; width: 200px }}</style>\
         <div class=flex><div id=i style=\"flex-basis: 0; margin-right: 10%\">XX</div><div id=n>X</div></div>"
    );
    let ((x, _, width, _), _) = boxes(&html, "i");
    let ((nx, _, _, _), _) = boxes(&html, "n");
    assert_close(width, 40.0, "border-box width", &html);
    assert_close(nx - x, 60.0, "next item offset", &html);
}

#[test]
fn percent_padding_of_a_flex_item_container_counts_in_its_automatic_minimum_size() {
    // The item lays out its own children, as a block, flex or grid
    // container; its 10% padding still refers to the 200px flex container.
    for display in ["block", "flex", "grid"] {
        let html = format!(
            "<!DOCTYPE html><style>{PAGE} body {{ font: 20px/1 Ahem }} p {{ margin: 0 }}\
             .flex {{ display: flex; width: 200px }}</style>\
             <div class=flex><div id=i style=\"display: {display}; flex-basis: 0; \
             padding-right: 10%; padding-left: 5%\"><p>XX</p></div></div>"
        );
        let ((_, _, width, _), (_, _, cw, _)) = boxes(&html, "i");
        assert_close(width, 70.0, &format!("{display} border-box width"), &html);
        assert_close(cw, 40.0, &format!("{display} content width"), &html);
    }
}

#[test]
fn percent_padding_of_a_vertical_flex_item_refers_to_the_container_height() {
    // A column flex container in a vertical writing mode measures its items
    // without a physical height, the inline size their percentage edges
    // refer to: 10% of the 150px container adds 15px to the 40px text.
    let html = format!(
        "<!DOCTYPE html><style>{PAGE} body {{ font: 20px/1 Ahem }}\
         .flex {{ display: flex; flex-direction: column; writing-mode: vertical-rl; height: 150px }}</style>\
         <div class=flex><div id=i style=\"flex-basis: 0; padding-bottom: 10%\">XX</div></div>"
    );
    let ((_, _, _, height), _) = boxes(&html, "i");
    assert_close(height, 55.0, "border-box height", &html);
}
