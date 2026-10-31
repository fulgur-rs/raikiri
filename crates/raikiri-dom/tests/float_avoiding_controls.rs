//! A block-level replaced element keeps its border box clear of the margin
//! boxes of the floats in its formatting context (CSS 2.1 section 9.5): it
//! sits beside a float when it fits in the room the float leaves, and below it
//! when it does not. Browsers lay out inline SVG roots and form controls such
//! as `<input>`, `<select>`, `<textarea>` and `<button>`, and embedded content
//! such as `<video>`, `<iframe>` and `<object>`, the same way, whatever their
//! size.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with no margin on the body. The
//! elements carry no padding, border or margin of their own, so the user agent
//! style of a control does not change the sizes.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

/// The markup of the container: a float of 100px by 100px, then `element`.
fn markup(element: &str) -> String {
    format!(
        "<div id=o style=\"width:200px\"><div style=\"float:left;width:100px;height:100px\"></div>{element}</div>"
    )
}

/// Checks the border boxes of the container `o` and of the element `c`.
fn check_both(element: &str, c: Rect, o: Rect) {
    check(&markup(element), &[("c", c), ("o", o)]);
}

#[test]
fn an_inline_svg_root_beside_a_float_when_it_fits() {
    check_both(
        "<svg id=c width=1 height=1 style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></svg>",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn an_inline_svg_root_below_a_float_when_it_does_not_fit() {
    check_both(
        "<svg id=c width=1 height=1 style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></svg>",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn a_text_input_beside_a_float_when_it_fits() {
    check_both(
        "<input id=c style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\">",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_text_input_below_a_float_when_it_does_not_fit() {
    check_both(
        "<input id=c style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\">",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn a_file_input_beside_a_float_when_it_fits() {
    check_both(
        "<input id=c type=file style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\">",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_file_input_below_a_float_when_it_does_not_fit() {
    check_both(
        "<input id=c type=file style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\">",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn an_image_input_beside_a_float_when_it_fits() {
    check_both(
        "<input id=c type=image style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\">",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn an_image_input_below_a_float_when_it_does_not_fit() {
    check_both(
        "<input id=c type=image style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\">",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn a_select_beside_a_float_when_it_fits() {
    check_both(
        "<select id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></select>",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_select_below_a_float_when_it_does_not_fit() {
    check_both(
        "<select id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></select>",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn a_textarea_beside_a_float_when_it_fits() {
    check_both(
        "<textarea id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></textarea>",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_textarea_below_a_float_when_it_does_not_fit() {
    check_both(
        "<textarea id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></textarea>",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn a_button_beside_a_float_when_it_fits() {
    check_both(
        "<button id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></button>",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_button_below_a_float_when_it_does_not_fit() {
    check_both(
        "<button id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></button>",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn a_video_beside_a_float_when_it_fits() {
    check_both(
        "<video id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></video>",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_video_below_a_float_when_it_does_not_fit() {
    check_both(
        "<video id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></video>",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn an_audio_player_beside_a_float_when_it_fits() {
    check_both(
        "<audio id=c controls style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></audio>",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn an_audio_player_below_a_float_when_it_does_not_fit() {
    check_both(
        "<audio id=c controls style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></audio>",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn an_iframe_beside_a_float_when_it_fits() {
    check_both(
        "<iframe id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></iframe>",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn an_iframe_below_a_float_when_it_does_not_fit() {
    check_both(
        "<iframe id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></iframe>",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn an_image_without_a_source_beside_a_float_when_it_fits() {
    check_both(
        "<img id=c style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\">",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn an_image_without_a_source_below_a_float_when_it_does_not_fit() {
    check_both(
        "<img id=c style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\">",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn a_canvas_beside_a_float_when_it_fits() {
    check_both(
        "<canvas id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></canvas>",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_canvas_below_a_float_when_it_does_not_fit() {
    check_both(
        "<canvas id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></canvas>",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn a_meter_narrow_overlaps_a_float() {
    check_both(
        "<meter id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></meter>",
        (0.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_meter_wide_overlaps_a_float() {
    check_both(
        "<meter id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></meter>",
        (0.0, 0.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_progress_bar_narrow_overlaps_a_float() {
    check_both(
        "<progress id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></progress>",
        (0.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_progress_bar_wide_overlaps_a_float() {
    check_both(
        "<progress id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></progress>",
        (0.0, 0.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_details_block_narrow_overlaps_a_float() {
    check_both(
        "<details id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\"></details>",
        (0.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_details_block_wide_overlaps_a_float() {
    check_both(
        "<details id=c  style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\"></details>",
        (0.0, 0.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn an_inline_svg_root_with_auto_margins_centres_in_the_room_beside_a_float() {
    check_both(
        "<svg id=c width=40 height=20 style=\"display:block;margin:0 auto\"></svg>",
        (130.0, 0.0, 40.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn a_button_with_auto_margins_centres_in_the_room_beside_a_float() {
    check_both(
        "<button id=c style=\"display:block;padding:0;border:0;height:20px;width:40px;margin:0 auto\"></button>",
        (130.0, 0.0, 40.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn an_inline_svg_root_with_clear_moves_below_a_float() {
    check_both(
        "<svg id=c width=40 height=20 style=\"display:block;clear:left\"></svg>",
        (0.0, 100.0, 40.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn an_embed_beside_a_float_when_it_fits() {
    check_both(
        "<embed id=c src=\"missing.png\" style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\">",
        (100.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn an_embed_below_a_float_when_it_does_not_fit() {
    check_both(
        "<embed id=c src=\"missing.png\" style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\">",
        (0.0, 100.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 120.0),
    );
}

#[test]
fn an_object_with_fallback_text_narrow_overlaps_a_float() {
    check_both(
        "<object id=c style=\"display:block;padding:0;border:0;margin:0;height:20px;width:50px\">XX</object>",
        (0.0, 0.0, 50.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}

#[test]
fn an_object_with_fallback_text_wide_overlaps_a_float() {
    check_both(
        "<object id=c style=\"display:block;padding:0;border:0;margin:0;height:20px;width:150px\">XX</object>",
        (0.0, 0.0, 150.0, 20.0),
        (0.0, 0.0, 200.0, 20.0),
    );
}
