//! `align-content` on block containers whose content is not a horizontal
//! block flow (CSS Box Alignment 3 §5.1): vertical writing modes shift the
//! content along the horizontal block axis, a multi-column container with
//! inline content shifts its lines, and an HTML button keeps its centered
//! content when it is given a table display, because a button ignores table
//! displays (HTML §15.5.13 "Button layout"). The positions are the ones
//! Chromium 153 gives for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf");

/// The border box `(x, y, width, height)` of the element with `id`.
fn rect(body: &str, id: &str) -> (f32, f32, f32, f32) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let document = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("layout must complete")
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let fragment = page
        .fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .expect("the element has a fragment");
    let r = fragment.rect();
    (r.x, r.y, r.width, r.height)
}

/// The left edge of `<span id=s>` in a vertical container, relative to the
/// same container without content alignment, so the shift is measured on its
/// own.
fn span_shift(style: &str, alignment: &str) -> f32 {
    let at = |alignment: &str| {
        rect(
            &format!("<div style='{style};align-content:{alignment}'><span id=s>ab</span></div>"),
            "s",
        )
        .0
    };
    at(alignment) - at("normal")
}

#[test]
fn vertical_inline_content_moves_along_the_horizontal_block_axis() {
    // Chromium puts the span at x 10 when centered and at x 20 (vertical-rl)
    // or x 0 (vertical-lr) without alignment.
    let rl = "height:30px;width:30px;writing-mode:vertical-rl";
    assert_eq!(span_shift(rl, "center"), -10.0);
    let lr = "height:30px;width:30px;writing-mode:vertical-lr";
    assert_eq!(span_shift(lr, "center"), 10.0);
    let wide = "height:30px;width:40px;writing-mode:vertical-rl";
    assert_eq!(span_shift(wide, "end"), -30.0);
}

#[test]
fn vertical_block_children_move_along_the_horizontal_block_axis() {
    let centered = "<div style='height:30px;width:30px;writing-mode:vertical-rl;\
                    align-content:center'><div id=d>ab</div></div>";
    assert_eq!(rect(centered, "d"), (10.0, 0.0, 10.0, 30.0));
    let padded = "<div style='height:30px;width:40px;writing-mode:vertical-lr;\
                  align-content:end;padding:2px 3px'><div id=d>ab</div></div>";
    assert_eq!(rect(padded, "d"), (27.0, 2.0, 10.0, 26.0));
}

#[test]
fn multicol_inline_content_is_aligned_in_the_column() {
    let body = "<div style='height:30px;columns:2;align-content:center'><span id=s>ab</span></div>";
    assert_eq!(rect(body, "s"), (0.0, 10.0, 20.0, 10.0));
    let button = "<button id=c style='all:unset;display:block;height:30px;columns:2'>\
                  <span id=s>ab</span></button>";
    assert_eq!(rect(button, "s"), (0.0, 10.0, 20.0, 10.0));
}

#[test]
fn a_button_with_a_table_display_keeps_its_centered_content() {
    for display in [
        "table",
        "table-cell",
        "inline-table",
        "table-row",
        "table-caption",
    ] {
        let body = format!(
            "<button id=c style='all:unset;display:{display};height:30px'><span id=s>ab</span></button>"
        );
        assert_eq!(rect(&body, "c"), (0.0, 0.0, 20.0, 30.0), "{display}");
        assert_eq!(rect(&body, "s"), (0.0, 10.0, 20.0, 10.0), "{display}");
    }
    // Two adjacent table-cell buttons do not form a table row: each is its
    // own block.
    let pair = "<div><button style='all:unset;display:table-cell;height:30px;width:40px'>\
                <span>ab</span></button><button id=e style='all:unset;display:table-cell;\
                height:30px'><span id=f>cd</span></button></div>";
    assert_eq!(rect(pair, "e"), (0.0, 30.0, 20.0, 30.0));
    assert_eq!(rect(pair, "f"), (0.0, 40.0, 20.0, 10.0));
}

#[test]
fn a_table_that_is_not_a_button_ignores_content_alignment() {
    let body =
        "<div style='display:table;height:30px;align-content:center'><span id=s>ab</span></div>";
    assert_eq!(rect(body, "s"), (0.0, 0.0, 20.0, 10.0));
}
