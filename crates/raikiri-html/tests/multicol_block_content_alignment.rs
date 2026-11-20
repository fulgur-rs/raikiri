//! `align-content` on a multi-column container whose content holds block
//! boxes (CSS Box Alignment 3 §5.1): the rows of columns move as a whole
//! within the content box, by the room their content leaves. The positions
//! are the ones Chromium 153 gives for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf");

/// The border boxes `(x, y, width, height)` of the fragments of the element
/// with `id` on the first page.
fn rects(body: &str, id: &str) -> Vec<(f32, f32, f32, f32)> {
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
    page.fragments()
        .filter(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .map(|fragment| {
            let r = fragment.rect();
            (r.x, r.y, r.width, r.height)
        })
        .collect()
}

/// The block offset of the first fragment of the element with `id`.
fn top(body: &str, id: &str) -> f32 {
    rects(body, id)
        .first()
        .expect("the element has a fragment")
        .1
}

/// A two-column container 100px wide with no gap, with `style` added.
fn columns(style: &str, content: &str) -> String {
    format!("<div style='width:100px;columns:2;column-gap:0;{style}'>{content}</div>")
}

#[test]
fn balanced_columns_move_by_the_room_their_content_leaves() {
    // One line balances to columns 10px tall.
    let end = columns("height:30px;align-content:end", "<div id=d>ab</div>");
    assert_eq!(top(&end, "d"), 20.0);
    let center = columns("height:40px;align-content:center", "<div id=d>a b c</div>");
    assert_eq!(rects(&center, "d"), vec![(0.0, 15.0, 50.0, 10.0)]);
    // Three blocks balance to two in the first column and one in the second.
    let blocks = columns(
        "height:60px;align-content:end",
        "<div id=a>a</div><div id=b>b</div><div id=c>c</div>",
    );
    assert_eq!(rects(&blocks, "a"), vec![(0.0, 40.0, 50.0, 10.0)]);
    assert_eq!(rects(&blocks, "b"), vec![(0.0, 50.0, 50.0, 10.0)]);
    assert_eq!(rects(&blocks, "c"), vec![(50.0, 40.0, 50.0, 10.0)]);
    // Boxes without lines are cut between the columns where they end.
    let boxes = columns(
        "height:60px;align-content:center",
        "<div id=a style='height:10px'></div><div id=b style='height:10px'></div>",
    );
    assert_eq!(rects(&boxes, "a"), vec![(0.0, 25.0, 50.0, 10.0)]);
    assert_eq!(rects(&boxes, "b"), vec![(50.0, 25.0, 50.0, 10.0)]);
    // A forced break gives each column one line.
    let forced = columns(
        "height:60px;align-content:end",
        "<div id=a>a</div><div id=b style='break-before:column'>b</div>",
    );
    assert_eq!(rects(&forced, "b"), vec![(50.0, 50.0, 50.0, 10.0)]);
}

#[test]
fn filled_columns_move_by_the_room_below_their_tallest_column() {
    let one = "<div style='height:30px;columns:2;column-fill:auto;align-content:center'>\
               <div id=d>ab</div></div>";
    assert_eq!(top(one, "d"), 10.0);
    let three = columns(
        "height:40px;column-fill:auto;align-content:end",
        "<div id=d>a b c d e f g h i</div>",
    );
    assert_eq!(rects(&three, "d"), vec![(0.0, 10.0, 50.0, 30.0)]);
}

#[test]
fn the_rows_and_spanners_move_together() {
    let body = columns(
        "height:60px;align-content:end",
        "<div id=a>a b</div><div id=s style='column-span:all'>s</div><div id=b>c d</div>",
    );
    assert_eq!(top(&body, "a"), 30.0);
    assert_eq!(top(&body, "s"), 40.0);
    assert_eq!(top(&body, "b"), 50.0);
}

#[test]
fn the_content_box_and_the_minimum_height_give_the_room() {
    let padded = columns(
        "height:50px;padding:5px;align-content:end",
        "<div id=d>a b c</div>",
    );
    assert_eq!(rects(&padded, "d"), vec![(5.0, 25.0, 45.0, 20.0)]);
    let minimum = columns("min-height:40px;align-content:end", "<div id=d>a b</div>");
    assert_eq!(top(&minimum, "d"), 30.0);
    // A wrapper is cut between the columns with the paragraph in it.
    let wrapped = columns(
        "height:60px;align-content:end",
        "<div id=w style='padding:5px'><div id=d>a b c</div></div>",
    );
    assert_eq!(top(&wrapped, "w"), 35.0);
    assert_eq!(rects(&wrapped, "d")[0], (5.0, 40.0, 40.0, 20.0));
}

#[test]
fn a_trailing_margin_is_left_out_and_a_leading_one_kept() {
    let trailing = columns(
        "height:40px;align-content:end",
        "<div id=d style='margin-bottom:10px'>a</div>",
    );
    assert_eq!(top(&trailing, "d"), 30.0);
    let leading = columns(
        "height:40px;align-content:end",
        "<div id=d style='margin-top:10px'>a</div>",
    );
    assert_eq!(top(&leading, "d"), 30.0);
    // Balanced columns of an auto height leave no room, whatever margin
    // they end with.
    let auto = columns(
        "align-content:end",
        "<div id=d style='margin-bottom:10px'>a</div>",
    );
    assert_eq!(top(&auto, "d"), 0.0);
}

#[test]
fn overflowing_columns_stay_at_the_start() {
    let text = columns(
        "height:20px;align-content:center",
        "<div id=d>a b c d e f</div>",
    );
    assert_eq!(top(&text, "d"), 0.0);
    let filled = columns(
        "height:20px;column-fill:auto;align-content:end",
        "<div id=a>a</div><div id=b>b</div><div id=c>c</div>",
    );
    assert_eq!(rects(&filled, "c"), vec![(50.0, 0.0, 50.0, 10.0)]);
    // Distributed alignment falls back to the start.
    let between = columns(
        "height:40px;align-content:space-between",
        "<div id=d>a</div>",
    );
    assert_eq!(top(&between, "d"), 0.0);
    // Balanced columns that size the container leave no room to move into,
    // even with unsafe alignment.
    let unsafe_end = columns("align-content:unsafe end", "<div id=d>a</div>");
    assert_eq!(rects(&unsafe_end, "d"), vec![(0.0, 0.0, 50.0, 10.0)]);
    let unsafe_center = columns("align-content:unsafe center", "<div id=d>a</div>");
    assert_eq!(rects(&unsafe_center, "d"), vec![(0.0, 0.0, 50.0, 10.0)]);
    let minimum = columns(
        "min-height:30px;align-content:unsafe end",
        "<div id=d>a</div>",
    );
    assert_eq!(top(&minimum, "d"), 20.0);
}

#[test]
fn floats_and_positioned_boxes_follow_the_moved_content() {
    let float = columns(
        "height:40px;align-content:end",
        "<div id=f style='float:left;width:10px;height:10px'></div><div id=d>a</div>",
    );
    assert_eq!(top(&float, "f"), 30.0);
    assert_eq!(top(&float, "d"), 30.0);
    // An inset resolves against the container; a static position follows
    // the content before it.
    let positioned = columns(
        "position:relative;height:40px;align-content:end",
        "<div id=d>a</div><div id=p style='position:absolute;top:0'>p</div>\
         <div id=q style='position:absolute'>q</div>",
    );
    assert_eq!(top(&positioned, "d"), 30.0);
    assert_eq!(top(&positioned, "p"), 0.0);
    assert_eq!(top(&positioned, "q"), 40.0);
    // A static position before every box in flow is at the aligned start.
    let leading = columns(
        "position:relative;height:40px;align-content:end",
        "<div id=q style='position:absolute'>q</div><div id=a>a</div><div id=b>b</div>",
    );
    assert_eq!(top(&leading, "q"), 30.0);
    assert_eq!(rects(&leading, "b"), vec![(50.0, 30.0, 50.0, 10.0)]);
}

#[test]
fn percentage_heights_resolve_against_the_unmoved_content_box() {
    // Half of the 100px content box is 50px of content, cut into two
    // 25px fragments whether or not the columns move.
    let normal = columns(
        "height:100px",
        "<div id=d style='height:50%;min-height:1px'></div>",
    );
    let at_end = columns(
        "height:100px;align-content:end",
        "<div id=d style='height:50%;min-height:1px'></div>",
    );
    assert_eq!(
        rects(&normal, "d"),
        vec![(0.0, 0.0, 50.0, 25.0), (50.0, 0.0, 50.0, 25.0)]
    );
    assert_eq!(
        rects(&at_end, "d"),
        vec![(0.0, 75.0, 50.0, 25.0), (50.0, 75.0, 50.0, 25.0)]
    );
}
