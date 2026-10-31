//! A block with an `auto` width fills the content box of its containing block
//! (CSS 2.1 section 10.3.3). When the containing block is itself an `auto`
//! width box whose width is not the one of the box around it (it shrinks to
//! fit its content, or leaves room for its own margins), the block takes the
//! width of that box, not the one of the nearest ancestor with a `width`.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px.

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
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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

#[test]
fn an_inline_block_holding_a_block_fits_the_block_when_its_parent_has_a_width() {
    // The inline-block takes the 40px of the text in the block, not the 300px of
    // its parent.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (8.0, 8.0, 40.0, 10.0)),
            ("k", (8.0, 8.0, 40.0, 10.0)),
        ],
    );
}

#[test]
fn a_float_holding_a_block_fits_the_block_when_its_parent_has_a_width() {
    // Same for a float.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"float:left\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 0.0)),
            ("c", (8.0, 8.0, 40.0, 10.0)),
            ("k", (8.0, 8.0, 40.0, 10.0)),
        ],
    );
}

#[test]
fn blocks_nested_in_an_inline_block_stay_inside_it() {
    // The width reaches the block inside the block.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block\"><div id=k><div id=l>YYYY</div></div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (8.0, 8.0, 40.0, 10.0)),
            ("k", (8.0, 8.0, 40.0, 10.0)),
            ("l", (8.0, 8.0, 40.0, 10.0)),
        ],
    );
}

#[test]
fn blocks_side_by_side_in_an_inline_block_take_the_widest_content() {
    // The wider block sets the width of the inline-block and the narrower one
    // stretches to it.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block\"><div id=k>YYYY</div><div id=m>YYYYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 20.0)),
            ("c", (8.0, 8.0, 60.0, 20.0)),
            ("k", (8.0, 8.0, 60.0, 10.0)),
            ("m", (8.0, 18.0, 60.0, 10.0)),
        ],
    );
}

#[test]
fn a_scroll_container_block_in_an_inline_block_fits_its_content() {
    // A block that is a scroll container is sized as any other block in it.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block\"><div id=k style=\"overflow:hidden\">YYYY</div></div></div>",
        &[("c", (8.0, 8.0, 40.0, 10.0)), ("k", (8.0, 8.0, 40.0, 10.0))],
    );
}

#[test]
fn an_inline_block_with_padding_holds_a_block_of_the_content_width() {
    // The block fits the content box of the inline-block, which is the 40px of the
    // text; the padding comes on top.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block;padding:0 10px\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (8.0, 8.0, 60.0, 10.0)),
            ("k", (18.0, 8.0, 40.0, 10.0)),
        ],
    );
}

#[test]
fn a_float_with_a_border_holds_a_block_of_the_content_width() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"float:left;border:5px solid\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 0.0)),
            ("c", (8.0, 8.0, 50.0, 20.0)),
            ("k", (13.0, 13.0, 40.0, 10.0)),
        ],
    );
}

#[test]
fn a_block_in_a_fit_content_box_takes_the_width_of_the_box() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"width:fit-content\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (8.0, 8.0, 40.0, 10.0)),
            ("k", (8.0, 8.0, 40.0, 10.0)),
        ],
    );
}

#[test]
fn an_inline_block_with_a_percentage_width_holds_a_block_of_that_width() {
    // A percentage of the 300px parent, not the content, decides the width.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block;width:50%\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (8.0, 8.0, 150.0, 10.0)),
            ("k", (8.0, 8.0, 150.0, 10.0)),
        ],
    );
}

#[test]
fn an_inline_block_with_a_length_width_holds_a_block_of_that_width() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block;width:100px\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (8.0, 8.0, 100.0, 10.0)),
            ("k", (8.0, 8.0, 100.0, 10.0)),
        ],
    );
}

#[test]
fn an_inline_block_that_wraps_its_text_holds_a_block_as_wide_as_itself() {
    // The text is longer than the 300px of the parent, so the inline-block fills
    // the parent and the text wraps.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block\"><div id=k>YYYYYYYY YYYYYYYY YYYYYYYY YYYYYYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 20.0)),
            ("c", (8.0, 8.0, 300.0, 20.0)),
            ("k", (8.0, 8.0, 300.0, 20.0)),
        ],
    );
}

#[test]
fn an_inline_block_that_fits_its_text_in_the_parent_holds_a_block_as_wide_as_the_text() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block\"><div id=k>YYYY ZZZZ</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (8.0, 8.0, 90.0, 10.0)),
            ("k", (8.0, 8.0, 90.0, 10.0)),
        ],
    );
}

#[test]
fn an_inline_block_inside_an_inline_box_holds_a_block_of_its_own_width() {
    // The inline box has no width of its own to give.
    check(
        "<div id=o style=\"width:300px\"><span id=s><div id=c style=\"display:inline-block\"><div id=k>YYYY</div></div></span></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("s", (8.0, 8.0, 40.0, 10.0)),
            ("c", (8.0, 8.0, 40.0, 10.0)),
            ("k", (8.0, 8.0, 40.0, 10.0)),
        ],
    );
}

#[test]
fn an_inline_block_inside_a_block_with_margins_holds_a_block_of_its_own_width() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"margin:0 10px\"><div id=d style=\"display:inline-block\"><div id=k>YYYY</div></div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (18.0, 8.0, 280.0, 10.0)),
            ("d", (18.0, 8.0, 40.0, 10.0)),
            ("k", (18.0, 8.0, 40.0, 10.0)),
        ],
    );
}

#[test]
fn a_block_with_margins_holds_a_block_that_leaves_room_for_them() {
    // The block has an `auto` width, which leaves room for its margins, and so
    // does the block inside it.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"margin:0 10px\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (18.0, 8.0, 280.0, 10.0)),
            ("k", (18.0, 8.0, 280.0, 10.0)),
        ],
    );
}

#[test]
fn a_list_item_with_margins_holds_a_block_that_leaves_room_for_them() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:list-item;margin:0 10px\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (18.0, 8.0, 280.0, 10.0)),
            ("k", (18.0, 8.0, 280.0, 10.0)),
        ],
    );
}

#[test]
fn a_flow_root_with_margins_holds_a_block_that_leaves_room_for_them() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:flow-root;margin:0 10px\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (18.0, 8.0, 280.0, 10.0)),
            ("k", (18.0, 8.0, 280.0, 10.0)),
        ],
    );
}

#[test]
fn a_relatively_positioned_block_with_padding_holds_a_block_of_the_content_width() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"position:relative;padding:0 10px\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (8.0, 8.0, 300.0, 10.0)),
            ("k", (18.0, 8.0, 280.0, 10.0)),
        ],
    );
}

#[test]
fn a_block_with_padding_holds_a_block_of_the_content_width() {
    // The control case: the block stretches and the padding comes off.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"padding:0 10px\"><div id=k>YYYY</div></div></div>",
        &[
            ("o", (8.0, 8.0, 300.0, 10.0)),
            ("c", (8.0, 8.0, 300.0, 10.0)),
            ("k", (18.0, 8.0, 280.0, 10.0)),
        ],
    );
}

#[test]
fn a_percentage_width_block_in_a_float_refers_to_the_float() {
    // Half of the 40px of the text, not of the 300px of the parent.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"float:left\"><div id=p style=\"width:50%\"><div id=k>YYYY</div></div></div></div>",
        &[
            ("c", (8.0, 8.0, 40.0, 10.0)),
            ("p", (8.0, 8.0, 20.0, 10.0)),
            ("k", (8.0, 8.0, 20.0, 10.0)),
        ],
    );
}

#[test]
fn a_calc_width_block_in_an_inline_block_refers_to_the_inline_block() {
    // The wider sibling sets the 60px of the inline-block; the width of the
    // other block is half of it and 10px more.
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"display:inline-block\"><div id=p style=\"width:calc(50% + 10px)\"><div id=k>YYYY</div></div><div id=m>YYYYYY</div></div></div>",
        &[
            ("c", (8.0, 8.0, 60.0, 20.0)),
            ("p", (8.0, 8.0, 40.0, 10.0)),
            ("k", (8.0, 8.0, 40.0, 10.0)),
            ("m", (8.0, 18.0, 60.0, 10.0)),
        ],
    );
}

#[test]
fn a_length_width_block_in_a_float_holds_a_block_of_that_width() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"float:left\"><div id=p style=\"width:100px\"><div id=k>YYYY</div></div></div></div>",
        &[
            ("c", (8.0, 8.0, 100.0, 10.0)),
            ("p", (8.0, 8.0, 100.0, 10.0)),
            ("k", (8.0, 8.0, 100.0, 10.0)),
        ],
    );
}

#[test]
fn a_block_inside_display_contents_in_a_float_fits_the_float() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"float:left\"><div style=\"display:contents\"><div id=k>YYYY</div></div></div></div>",
        &[("c", (8.0, 8.0, 40.0, 10.0)), ("k", (8.0, 8.0, 40.0, 10.0))],
    );
}

#[test]
fn a_block_in_nested_floats_fits_the_innermost() {
    check(
        "<div id=o style=\"width:300px\"><div id=c style=\"float:left\"><div id=d style=\"float:left\"><div id=k>YYYY</div></div></div></div>",
        &[
            ("c", (8.0, 8.0, 40.0, 10.0)),
            ("d", (8.0, 8.0, 40.0, 10.0)),
            ("k", (8.0, 8.0, 40.0, 10.0)),
        ],
    );
}
