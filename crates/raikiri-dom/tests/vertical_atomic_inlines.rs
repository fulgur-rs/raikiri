//! An atomic inline in a vertical paragraph shrinks to fit the room its line
//! offers along the inline axis, the physical height: its max-content size
//! when that fits, else the room, but never less than its min-content size
//! (CSS 2.1 §10.3.9, CSS Sizing 3 §5.1). A flex container that the room
//! squeezes shrinks its items.
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

/// Checks `content` under `vertical-lr` against `lr`, and under `vertical-rl`
/// against `rl`, both set on the root element.
fn check_vertical(content: &str, lr: &[(&str, Rect)], rl: &[(&str, Rect)]) {
    check(
        &format!("<html style=\"writing-mode:vertical-lr\">{content}"),
        lr,
    );
    check(
        &format!("<html style=\"writing-mode:vertical-rl\">{content}"),
        rl,
    );
}

const ITEMS: &str = "<div id=a>Black Panther</div><div id=b>Wonder Woman</div>\
                     <div id=c>Storm</div><div id=d>Flash</div>";

#[test]
fn a_vertical_inline_flex_shrinks_its_items_to_fit_the_room_of_its_line() {
    // The items and gaps take 410px of a 300px line: the container takes the
    // line, and the first two items give up the difference.
    let content = format!(
        "<div id=p style=\"height:300px\"><div id=s style=\"display:inline-flex;gap:20px\">{ITEMS}</div></div>"
    );
    check_vertical(
        &content,
        &[
            ("s", (8.0, 8.0, 20.0, 300.0)),
            ("a", (8.0, 8.0, 20.0, 72.8)),
            ("b", (8.0, 100.8, 20.0, 67.2)),
            ("c", (8.0, 188.0, 20.0, 50.0)),
            ("d", (8.0, 258.0, 20.0, 50.0)),
        ],
        &[
            ("s", (772.0, 8.0, 20.0, 300.0)),
            ("a", (772.0, 8.0, 20.0, 72.8)),
            ("b", (772.0, 100.8, 20.0, 67.2)),
            ("c", (772.0, 188.0, 20.0, 50.0)),
            ("d", (772.0, 258.0, 20.0, 50.0)),
        ],
    );
}

#[test]
fn a_vertical_inline_flex_that_fits_its_line_keeps_its_max_content_size() {
    let content = format!(
        "<div id=p style=\"height:500px\"><div id=s style=\"display:inline-flex;gap:20px\">{ITEMS}</div></div>"
    );
    check_vertical(
        &content,
        &[
            ("s", (8.0, 8.0, 10.0, 410.0)),
            ("a", (8.0, 8.0, 10.0, 130.0)),
            ("b", (8.0, 158.0, 10.0, 120.0)),
            ("c", (8.0, 298.0, 10.0, 50.0)),
            ("d", (8.0, 368.0, 10.0, 50.0)),
        ],
        &[
            ("s", (782.0, 8.0, 10.0, 410.0)),
            ("a", (782.0, 8.0, 10.0, 130.0)),
            ("b", (782.0, 158.0, 10.0, 120.0)),
            ("c", (782.0, 298.0, 10.0, 50.0)),
            ("d", (782.0, 368.0, 10.0, 50.0)),
        ],
    );
}

#[test]
fn a_vertical_inline_flex_takes_the_room_its_inline_margins_leave() {
    // 20px above and below leave 544px of the 584px line.
    let content = format!(
        "<div id=s style=\"display:inline-flex;margin:20px 0\"><div id=a>{}</div><div id=b>fghij</div></div>",
        "abcde ".repeat(60)
    );
    check_vertical(
        &content,
        &[
            ("s", (8.0, 28.0, 80.0, 544.0)),
            ("a", (8.0, 28.0, 80.0, 494.0)),
            ("b", (8.0, 522.0, 80.0, 50.0)),
        ],
        &[
            ("s", (712.0, 28.0, 80.0, 544.0)),
            ("a", (712.0, 28.0, 80.0, 494.0)),
            ("b", (712.0, 522.0, 80.0, 50.0)),
        ],
    );
}

#[test]
fn a_vertical_inline_flex_takes_at_most_the_line_whatever_its_content() {
    let content = format!(
        "<div id=s style=\"display:inline-flex;flex-wrap:wrap\"><div id=a>{}</div><div id=b>fghij</div></div>",
        "abcdefghi ".repeat(40)
    );
    check_vertical(
        &content,
        &[
            ("s", (8.0, 8.0, 90.0, 584.0)),
            ("a", (8.0, 8.0, 80.0, 584.0)),
            ("b", (88.0, 8.0, 10.0, 50.0)),
        ],
        &[
            ("s", (702.0, 8.0, 90.0, 584.0)),
            ("a", (712.0, 8.0, 80.0, 584.0)),
            ("b", (702.0, 8.0, 10.0, 50.0)),
        ],
    );
}

#[test]
fn a_vertical_inline_flex_in_a_vertical_inline_flex_shrinks_with_it() {
    let content = "<div id=p style=\"height:300px\"><div id=s style=\"display:inline-flex\">\
                   <div id=a style=\"display:inline-flex\"><div id=x>Black Panther</div>\
                   <div id=y>Wonder Woman</div></div><div id=b>Storm</div></div></div>";
    check_vertical(
        content,
        &[
            ("s", (8.0, 8.0, 10.0, 300.0)),
            ("a", (8.0, 8.0, 10.0, 250.0)),
            ("x", (8.0, 8.0, 10.0, 130.0)),
            ("y", (8.0, 138.0, 10.0, 120.0)),
            ("b", (8.0, 258.0, 10.0, 50.0)),
        ],
        &[
            ("s", (782.0, 8.0, 10.0, 300.0)),
            ("a", (782.0, 8.0, 10.0, 250.0)),
            ("x", (782.0, 8.0, 10.0, 130.0)),
            ("y", (782.0, 138.0, 10.0, 120.0)),
            ("b", (782.0, 258.0, 10.0, 50.0)),
        ],
    );
}

#[test]
fn a_vertical_inline_grid_shrinks_to_fit_the_room_of_its_line() {
    let grid = |height: u32| {
        format!(
            "<div id=p style=\"height:{height}px\"><div id=s style=\"display:inline-grid;\
             grid-template-columns:auto auto\"><div id=a>abcde fghij</div>\
             <div id=b>klmno pqrst</div></div></div>"
        )
    };
    // The room is below the min-content size of the grid: it keeps that.
    check_vertical(
        &grid(80),
        &[
            ("s", (8.0, 8.0, 20.0, 100.0)),
            ("a", (8.0, 8.0, 20.0, 50.0)),
            ("b", (8.0, 58.0, 20.0, 50.0)),
        ],
        &[
            ("s", (772.0, 8.0, 20.0, 100.0)),
            ("a", (772.0, 8.0, 20.0, 50.0)),
            ("b", (772.0, 58.0, 20.0, 50.0)),
        ],
    );
    // The room is between its min-content and max-content sizes: it takes it.
    check_vertical(
        &grid(130),
        &[
            ("s", (8.0, 8.0, 20.0, 130.0)),
            ("a", (8.0, 8.0, 20.0, 65.0)),
            ("b", (8.0, 73.0, 20.0, 65.0)),
        ],
        &[
            ("s", (772.0, 8.0, 20.0, 130.0)),
            ("a", (772.0, 8.0, 20.0, 65.0)),
            ("b", (772.0, 73.0, 20.0, 65.0)),
        ],
    );
    // The room fits its max-content size.
    check_vertical(
        &grid(400),
        &[
            ("s", (8.0, 8.0, 10.0, 220.0)),
            ("a", (8.0, 8.0, 10.0, 110.0)),
            ("b", (8.0, 118.0, 10.0, 110.0)),
        ],
        &[
            ("s", (782.0, 8.0, 10.0, 220.0)),
            ("a", (782.0, 8.0, 10.0, 110.0)),
            ("b", (782.0, 118.0, 10.0, 110.0)),
        ],
    );
}

#[test]
fn a_vertical_inline_block_takes_at_most_the_line_and_no_more_than_its_content() {
    check_vertical(
        "<div id=s style=\"display:inline-block\">abcde fghij</div>",
        &[("s", (8.0, 8.0, 10.0, 110.0))],
        &[("s", (782.0, 8.0, 10.0, 110.0))],
    );
    let long = format!(
        "<div id=s style=\"display:inline-block\">{}</div>",
        "abcde ".repeat(120)
    );
    check_vertical(
        &long,
        &[("s", (8.0, 8.0, 140.0, 584.0))],
        &[("s", (652.0, 8.0, 140.0, 584.0))],
    );
}

#[test]
fn a_vertical_inline_block_takes_the_room_its_inline_margins_leave() {
    // 30px above and below leave 524px of the 584px line. A margin on the
    // block axis takes room across the lines, not along them.
    let content = |margin: &str| {
        format!(
            "<div id=s style=\"display:inline-block;margin:{margin}\">{}</div>",
            "abcde ".repeat(120)
        )
    };
    check_vertical(
        &content("30px 0"),
        &[("s", (8.0, 38.0, 150.0, 524.0))],
        &[("s", (642.0, 38.0, 150.0, 524.0))],
    );
    check_vertical(
        &content("0 0 0 30px"),
        &[("s", (38.0, 8.0, 140.0, 584.0))],
        &[("s", (652.0, 8.0, 140.0, 584.0))],
    );
}

#[test]
fn the_blocks_of_a_vertical_inline_block_take_its_shrunk_size() {
    let content = format!(
        "<div id=p style=\"height:300px\"><div id=s style=\"display:inline-block\">\
         <div id=a>{}</div><div id=b>fghij</div></div></div>",
        "abcde ".repeat(20)
    );
    check_vertical(
        &content,
        &[
            ("s", (8.0, 8.0, 50.0, 300.0)),
            ("a", (8.0, 8.0, 40.0, 300.0)),
            ("b", (48.0, 8.0, 10.0, 300.0)),
        ],
        &[
            ("s", (742.0, 8.0, 50.0, 300.0)),
            ("a", (752.0, 8.0, 40.0, 300.0)),
            ("b", (742.0, 8.0, 10.0, 300.0)),
        ],
    );
}

#[test]
fn two_vertical_inline_blocks_each_take_their_own_size_in_one_line() {
    let content = "<div id=p style=\"height:300px\">\
                   <div id=s style=\"display:inline-block\">abcde fghij</div>\
                   <div id=t style=\"display:inline-block\">klmno pqrst uvwxy zabcd efghi jklmn opqrs tuvwx yz</div></div>";
    check_vertical(
        content,
        &[
            ("s", (8.0, 8.0, 10.0, 110.0)),
            ("t", (18.0, 8.0, 20.0, 300.0)),
        ],
        &[
            ("s", (782.0, 8.0, 10.0, 110.0)),
            ("t", (762.0, 8.0, 20.0, 300.0)),
        ],
    );
}

#[test]
fn a_horizontal_atomic_in_a_vertical_line_keeps_its_content_size() {
    check_vertical(
        "<div id=s style=\"display:inline-block;writing-mode:horizontal-tb\">abcde fghij</div>",
        &[("s", (8.0, 8.0, 110.0, 10.0))],
        &[("s", (682.0, 8.0, 110.0, 10.0))],
    );
    check_vertical(
        "<div id=s style=\"display:inline-flex;writing-mode:horizontal-tb\">\
         <div id=a>abcde</div><div id=b>fghij</div></div>",
        &[
            ("s", (8.0, 8.0, 100.0, 10.0)),
            ("a", (8.0, 8.0, 50.0, 10.0)),
            ("b", (58.0, 8.0, 50.0, 10.0)),
        ],
        &[
            ("s", (692.0, 8.0, 100.0, 10.0)),
            ("a", (692.0, 8.0, 50.0, 10.0)),
            ("b", (742.0, 8.0, 50.0, 10.0)),
        ],
    );
}

#[test]
fn a_vertical_inline_flex_in_a_horizontal_line_takes_the_page_less_its_margins() {
    // A horizontal line sets no limit on the height of the vertical box, so
    // the page height stands in for it, and the box takes what the margins
    // leave of it. The 790px of text wrap to two lines.
    let words = ["wwwwwwwww"; 8].join(" ");
    check(
        &format!(
            "<div id=p style=\"width:300px\"><span id=s \
             style=\"display:inline-flex;writing-mode:vertical-rl;margin:30px 0\">\
             <div id=a>{words}</div></span></div>"
        ),
        &[
            ("s", (8.0, 38.0, 20.0, 540.0)),
            ("a", (8.0, 38.0, 20.0, 540.0)),
        ],
    );
}

#[test]
fn a_float_around_a_vertical_atomic_measures_its_min_content_size() {
    // The line of the float is 34px, less than the 90px the text asks for
    // and than the 40px of its longest word: the float takes the word, and
    // so does the atomic inside it.
    for atomic in [
        "<span id=o style=\"display:inline-block\">aaaa bbbb</span>",
        "<span id=o style=\"display:inline-flex\"><div id=a>aaaa bbbb</div></span>",
    ] {
        check(
            &format!(
                "<html style=\"writing-mode:vertical-lr;height:50px\">\
                 <div id=f style=\"float:left\">{atomic}</div>"
            ),
            &[("f", (8.0, 8.0, 20.0, 40.0)), ("o", (8.0, 8.0, 20.0, 40.0))],
        );
    }
}
