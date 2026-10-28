//! An atomic inline in a vertical paragraph shrinks to fit the room its line
//! offers along the inline axis, the physical height: its max-content size
//! when that fits, else the room, but never less than its min-content size
//! (CSS 2.1 §10.3.9, CSS Sizing 3 §5.1). A flex container that the room
//! squeezes shrinks its items. The floats of a vertical paragraph are sized
//! against the same widths as its atomic inlines.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px. The
//! tests that fill the width of the page take the body margin away, so that
//! the page does not cut the boxes.

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

/// Checks `body`, where `MODE` stands for the vertical writing mode, against
/// `lr` under `vertical-lr` and against `rl` under `vertical-rl`.
fn check_modes(body: &str, lr: &[(&str, Rect)], rl: &[(&str, Rect)]) {
    check(&body.replace("MODE", "vertical-lr"), lr);
    check(&body.replace("MODE", "vertical-rl"), rl);
}

/// A float whose `aa` and inline-block `bbbb` (styled by `atomic_css`) set the
/// 20px and 40px they take along the line.
fn float_with_atomic(root_css: &str, atomic_css: &str) -> String {
    format!(
        "<div id=r style=\"float:left;writing-mode:MODE;{root_css}\">aa\
         <div id=a style=\"display:inline-block;{atomic_css}\">bbbb</div></div>"
    )
}

#[test]
fn a_percentage_height_atomic_is_measured_as_auto_and_resolved_against_the_lines() {
    // The float's height is not known while it is measured: the 50% counts as
    // `auto`, the 40px of text, and the float takes 20 + 40px. The percentage
    // then resolves against that height.
    let same = [
        ("a", (8.0, 28.0, 10.0, 30.0)),
        ("r", (8.0, 8.0, 10.0, 60.0)),
    ];
    check_modes(&float_with_atomic("", "height:50%"), &same, &same);
    // A float of a known height resolves it against that height.
    let known = [
        ("a", (8.0, 28.0, 10.0, 50.0)),
        ("r", (8.0, 8.0, 10.0, 100.0)),
    ];
    check_modes(
        &float_with_atomic("height:100px", "height:50%"),
        &known,
        &known,
    );
}

#[test]
fn a_percentage_height_block_child_is_measured_as_auto_in_a_vertical_paragraph() {
    // The block child of the paragraph is its inline extent tall; the 20px
    // is half of the 40px the text asks for.
    let content = |root_css: &str| {
        format!(
            "<div id=r style=\"float:left;writing-mode:MODE;{root_css}\">aa\
             <div id=a style=\"height:50%\">bbbb</div></div>"
        )
    };
    check_modes(
        &content(""),
        &[
            ("a", (18.0, 8.0, 10.0, 20.0)),
            ("r", (8.0, 8.0, 20.0, 40.0)),
        ],
        &[("a", (8.0, 8.0, 10.0, 20.0)), ("r", (8.0, 8.0, 20.0, 40.0))],
    );
    check_modes(
        &content("height:100px"),
        &[
            ("a", (18.0, 8.0, 10.0, 50.0)),
            ("r", (8.0, 8.0, 20.0, 100.0)),
        ],
        &[
            ("a", (8.0, 8.0, 10.0, 50.0)),
            ("r", (8.0, 8.0, 20.0, 100.0)),
        ],
    );
}

/// A root of `root_css` with the text `aa` and a horizontal inline-block
/// styled by `atomic_css` that holds `content`.
fn horizontal_atomic(root_css: &str, atomic_css: &str, content: &str) -> String {
    format!(
        "<div id=r style=\"writing-mode:MODE;{root_css}\">aa<div id=a \
         style=\"display:inline-block;writing-mode:horizontal-tb;{atomic_css}\">{content}</div></div>"
    )
}

#[test]
fn a_horizontal_atomic_in_a_vertical_root_fits_the_width_of_the_root() {
    // The 140px of text fit the 100px root in two lines, and so do they in a
    // 50px one in three.
    let words = "bb cc dd ee ff";
    let fits = [
        ("a", (8.0, 28.0, 100.0, 20.0)),
        ("r", (8.0, 8.0, 100.0, 300.0)),
    ];
    check_modes(
        &horizontal_atomic("width:100px;height:300px", "", words),
        &fits,
        &fits,
    );
    let narrow = [
        ("a", (8.0, 28.0, 50.0, 30.0)),
        ("r", (8.0, 8.0, 50.0, 300.0)),
    ];
    check_modes(
        &horizontal_atomic("width:50px;height:300px", "", words),
        &narrow,
        &narrow,
    );
    // The root's width follows the box when it is not known, and the box
    // takes all 140px.
    let auto = [
        ("a", (8.0, 28.0, 140.0, 10.0)),
        ("r", (8.0, 8.0, 140.0, 300.0)),
    ];
    check_modes(
        &horizontal_atomic("float:left;height:300px", "", words),
        &auto,
        &auto,
    );
}

#[test]
fn a_horizontal_atomic_in_a_vertical_root_leaves_room_for_its_horizontal_margins() {
    // The 5px and 11px to the sides are taken off the 100px, not the 3px and
    // 7px above and below, which lie along the line.
    let margins = [("a", (19.0, 31.0, 84.0, 20.0))];
    check_modes(
        &horizontal_atomic(
            "width:100px;height:300px",
            "margin:3px 5px 7px 11px",
            "bb cc dd ee ff",
        ),
        &margins,
        &margins,
    );
}

#[test]
fn a_horizontal_atomic_in_a_vertical_root_of_unknown_width_fits_the_page() {
    // The width of the root follows its content, so the page area stands in
    // for it: 800px of the 3000px of text, whatever the ancestors are, and
    // half of it for a 50% width.
    let long = "bb ".repeat(100);
    let page = [
        ("a", (8.0, 28.0, 800.0, 40.0)),
        ("r", (8.0, 8.0, 800.0, 100.0)),
    ];
    check_modes(
        &format!(
            "<div style=\"width:300px\">{}</div>",
            horizontal_atomic("float:left;height:100px", "", &long)
        ),
        &page,
        &page,
    );
    let half = [
        ("a", (8.0, 28.0, 400.0, 10.0)),
        ("r", (8.0, 8.0, 400.0, 100.0)),
    ];
    check_modes(
        &horizontal_atomic("float:left;height:100px", "width:50%", "bb cc dd"),
        &half,
        &half,
    );
}

#[test]
fn a_horizontal_atomic_resolves_a_percentage_width_against_the_width_of_the_root() {
    // The padding takes its 5% of the 200px the lines run along, the width
    // its 50% of the root's 100px.
    check_modes(
        &horizontal_atomic("width:100px;height:100px", "width:50%", "bb cc dd"),
        &[("a", (8.0, 28.0, 50.0, 20.0))],
        &[("a", (58.0, 28.0, 50.0, 20.0))],
    );
    check_modes(
        &horizontal_atomic(
            "width:100px;height:200px",
            "width:50%;padding:5%",
            "bb cc dd",
        ),
        &[("a", (8.0, 28.0, 50.0, 50.0))],
        &[("a", (58.0, 28.0, 50.0, 50.0))],
    );
}

#[test]
fn a_percentage_width_atomic_counts_as_auto_while_the_root_takes_its_width_from_its_lines() {
    let atomic = |css: &str| {
        format!(
            "<div id=r style=\"float:left;writing-mode:MODE;height:100px\">aa\
             <div id=a style=\"display:inline-block;{css}\">bb</div></div>"
        )
    };
    // The width of the line is that of the text, 10px, whatever the box asks
    // of it; its 50% does not become half the 100px height.
    let text = [
        ("a", (8.0, 28.0, 10.0, 20.0)),
        ("r", (8.0, 8.0, 10.0, 100.0)),
    ];
    for css in ["width:50%", "min-width:50%", "max-width:50%", ""] {
        check_modes(&atomic(css), &text, &text);
    }
    // The padding of the same box takes its percentage of the 100px.
    let padded = [
        ("a", (8.0, 28.0, 30.0, 40.0)),
        ("r", (8.0, 8.0, 30.0, 100.0)),
    ];
    check_modes(&atomic("width:50%;padding:10%"), &padded, &padded);
    // A box without content is as wide as its padding, whether or not it
    // states a percentage of the width it does not have yet.
    let empty = |css: &str| {
        format!(
            "<div id=r style=\"float:left;writing-mode:MODE;height:100px\">aa\
             <div id=a style=\"display:inline-block;height:10px;{css}\"></div></div>"
        )
    };
    let edges = [
        ("a", (8.0, 28.0, 20.0, 20.0)),
        ("r", (8.0, 8.0, 20.0, 100.0)),
    ];
    for css in [
        "padding:10%",
        "width:50%;padding:10%",
        "min-width:1%;padding:10%",
        "min-width:50%;padding:10%",
        "max-width:50%;padding:10%",
    ] {
        check_modes(&empty(css), &edges, &edges);
    }
}

#[test]
fn a_percentage_width_atomic_resolves_against_the_known_width_of_a_vertical_root() {
    let content = "<div id=r style=\"float:left;writing-mode:MODE;height:100px;width:80px\">aa\
                   <div id=a style=\"display:inline-block;width:50%;height:10px\"></div></div>";
    check_modes(
        content,
        &[("a", (8.0, 28.0, 40.0, 10.0))],
        &[("a", (48.0, 28.0, 40.0, 10.0))],
    );
}

#[test]
fn the_percentage_margins_of_an_atomic_in_a_vertical_root_follow_the_lines() {
    // 10% of the 100px height, whatever the width of the root.
    let content = "<div id=r style=\"float:left;writing-mode:MODE;height:100px\">aa\
                   <div id=a style=\"display:inline-block;width:10px;height:10px;margin:10%\"></div></div>";
    let same = [
        ("a", (18.0, 38.0, 10.0, 10.0)),
        ("r", (8.0, 8.0, 30.0, 100.0)),
    ];
    check_modes(content, &same, &same);
}

#[test]
fn a_percentage_height_horizontal_atomic_resolves_against_the_height_the_lines_gave() {
    // The 20px of text and the 10px of the box make the float 30px tall, and
    // the box takes half of that. With a known height of 100px it takes 50px.
    check_modes(
        &horizontal_atomic("float:left", "height:50%", "bb"),
        &[
            ("a", (18.0, 8.0, 20.0, 15.0)),
            ("r", (8.0, 8.0, 30.0, 30.0)),
        ],
        &[("a", (8.0, 8.0, 20.0, 15.0)), ("r", (8.0, 8.0, 30.0, 30.0))],
    );
    let known = [
        ("a", (8.0, 28.0, 20.0, 50.0)),
        ("r", (8.0, 8.0, 20.0, 100.0)),
    ];
    check_modes(
        &horizontal_atomic("float:left;height:100px", "height:50%", "bb"),
        &known,
        &known,
    );
}

/// A paragraph `aa` of the writing mode `MODE`, styled by `root_css`, with the
/// left float `f` (styled by `float_css`, holding `content`) in it.
fn paragraph_with_float(root_css: &str, float_css: &str, content: &str) -> String {
    format!(
        "<div id=r style=\"writing-mode:MODE;{root_css}\">aa\
         <div id=f style=\"float:left;{float_css}\">{content}</div></div>"
    )
}

/// A vertical paragraph of the known width of 100px and the height of 300px.
const KNOWN: &str = "width:100px;height:300px";

/// A vertical paragraph that is a float: the content of its lines decides its
/// width, and its height is 100px.
const FROM_LINES: &str = "float:left;height:100px";

#[test]
fn a_horizontal_float_in_a_vertical_root_fits_the_width_of_the_root() {
    // The text asks for 140px and the root is 100px wide: it takes two lines
    // of the width of the root, not of the 300px height of the paragraph.
    let fits = [
        ("f", (8.0, 8.0, 100.0, 20.0)),
        ("r", (8.0, 8.0, 100.0, 300.0)),
    ];
    check_modes(
        &paragraph_with_float(KNOWN, "writing-mode:horizontal-tb", "bb cc dd ee ff"),
        &fits,
        &fits,
    );
    // Its horizontal margins come off the room.
    let margins = [
        ("f", (18.0, 8.0, 70.0, 30.0)),
        ("r", (8.0, 8.0, 100.0, 300.0)),
    ];
    check_modes(
        &paragraph_with_float(
            KNOWN,
            "writing-mode:horizontal-tb;margin-left:10px;margin-right:20px",
            "bb cc dd ee ff",
        ),
        &margins,
        &margins,
    );
}

#[test]
fn a_horizontal_float_in_a_vertical_root_of_unknown_width_fits_the_page() {
    // The content of the root decides its width, so the page area stands in
    // for it: the text fits on one line and the root takes its width.
    let same = [
        ("f", (8.0, 8.0, 140.0, 10.0)),
        ("r", (8.0, 8.0, 140.0, 100.0)),
    ];
    check_modes(
        &paragraph_with_float(FROM_LINES, "writing-mode:horizontal-tb", "bb cc dd ee ff"),
        &same,
        &same,
    );
}

#[test]
fn the_percentage_widths_of_a_float_resolve_against_the_width_of_the_root() {
    let half = |lr_x: f32, rl_x: f32| {
        (
            [
                ("f", (lr_x, 8.0, 50.0, 20.0)),
                ("r", (8.0, 8.0, 100.0, 300.0)),
            ],
            [
                ("f", (rl_x, 8.0, 50.0, 20.0)),
                ("r", (8.0, 8.0, 100.0, 300.0)),
            ],
        )
    };
    let (lr, rl) = half(8.0, 58.0);
    // The 50% of the 100px, whether the float runs across the lines or along
    // them, and as a width or as a lower limit, also over a smaller width.
    for css in [
        "width:50%;height:20px",
        "writing-mode:horizontal-tb;width:50%;height:20px",
        "min-width:50%;height:20px",
        "writing-mode:horizontal-tb;min-width:50%;height:20px",
        "width:40%;min-width:50%;height:20px",
    ] {
        check_modes(&paragraph_with_float(KNOWN, css, ""), &lr, &rl);
    }
    // An upper limit, over a larger width.
    let capped_lr = [
        ("f", (8.0, 8.0, 30.0, 20.0)),
        ("r", (8.0, 8.0, 100.0, 300.0)),
    ];
    let capped_rl = [
        ("f", (78.0, 8.0, 30.0, 20.0)),
        ("r", (8.0, 8.0, 100.0, 300.0)),
    ];
    for css in [
        "width:80%;max-width:30%;height:20px",
        "width:80px;max-width:30%;height:20px",
    ] {
        check_modes(
            &paragraph_with_float(KNOWN, css, ""),
            &capped_lr,
            &capped_rl,
        );
    }
}

#[test]
fn the_percentage_widths_of_a_float_count_as_auto_while_the_root_takes_its_width_from_its_lines() {
    // The width of the root is that of the text, 10px, whatever the float asks
    // of it; its 50% does not become half the 100px height.
    let text = [
        ("f", (8.0, 8.0, 10.0, 20.0)),
        ("r", (8.0, 8.0, 10.0, 100.0)),
    ];
    check_modes(
        &paragraph_with_float(FROM_LINES, "width:50%;height:20px", "bb"),
        &text,
        &text,
    );
    // A lower limit that is a percentage is no limit: the float is empty.
    check_modes(
        &paragraph_with_float(FROM_LINES, "min-width:50%;height:20px", ""),
        &[("f", (8.0, 8.0, 0.0, 20.0)), ("r", (8.0, 8.0, 10.0, 100.0))],
        &[
            ("f", (18.0, 8.0, 0.0, 20.0)),
            ("r", (8.0, 8.0, 10.0, 100.0)),
        ],
    );
    // Its padding takes its percentage of the 100px all the same.
    let padded = [
        ("f", (8.0, 8.0, 20.0, 20.0)),
        ("r", (8.0, 8.0, 20.0, 100.0)),
    ];
    check_modes(
        &paragraph_with_float(FROM_LINES, "width:50%;padding:10%;height:10px", ""),
        &padded,
        &padded,
    );
    // A horizontal float takes the page area for the width of the root: 50% of
    // the 800px.
    let page = [
        ("f", (8.0, 8.0, 400.0, 20.0)),
        ("r", (8.0, 8.0, 400.0, 100.0)),
    ];
    check_modes(
        &paragraph_with_float(
            FROM_LINES,
            "writing-mode:horizontal-tb;width:50%;height:20px",
            "",
        ),
        &page,
        &page,
    );
}

#[test]
fn the_limits_of_an_atomic_resolve_against_the_known_width_of_a_vertical_root() {
    let atomic = |css: &str, content: &str| {
        format!(
            "<div id=r style=\"writing-mode:MODE;{KNOWN}\">aa\
             <div id=a style=\"display:inline-block;{css}\">{content}</div></div>"
        )
    };
    let half_lr = [
        ("a", (8.0, 28.0, 50.0, 10.0)),
        ("r", (8.0, 8.0, 100.0, 300.0)),
    ];
    let half_rl = [
        ("a", (58.0, 28.0, 50.0, 10.0)),
        ("r", (8.0, 8.0, 100.0, 300.0)),
    ];
    // 50% of the 100px width of the root, not of the 300px height.
    for css in [
        "min-width:50%;height:10px",
        "width:40%;min-width:50%;height:10px",
        "writing-mode:horizontal-tb;min-width:50%;height:10px",
    ] {
        check_modes(&atomic(css, ""), &half_lr, &half_rl);
    }
    check_modes(
        &atomic("max-width:30%", "bbbbbbbb"),
        &[("a", (8.0, 28.0, 10.0, 80.0))],
        &[("a", (98.0, 28.0, 10.0, 80.0))],
    );
}

/// Sets the root element to `MODE` and takes the body margin away, so that a
/// box as wide as the 800px page fits it.
const PAGE_MODE: &str = "<style>body{margin:0}html{writing-mode:MODE}</style>";

/// Twelve words of 90px with a space of 10px between them: 1190px of text.
fn twelve_words() -> String {
    ["wwwwwwwww"; 12].join(" ")
}

/// A horizontal inline-block holding `content`, with `css`.
fn horizontal_box(id: &str, css: &str, content: &str) -> String {
    format!(
        "<div id={id} style=\"display:inline-block;writing-mode:horizontal-tb;{css}\">\
         {content}</div>"
    )
}

#[test]
fn a_horizontal_inline_block_in_a_vertical_paragraph_wraps_at_the_page_width() {
    // The paragraph is 600px high and its width follows its lines, so the
    // page area stands in for the width the box fits: the 1190px of text
    // break in two lines of 800px, not in as many lines as 600px would ask.
    let wrapped = [
        ("p", (0.0, 0.0, 800.0, 600.0)),
        ("o", (0.0, 0.0, 800.0, 20.0)),
    ];
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p>{}</div>",
            horizontal_box("o", "", &twelve_words())
        ),
        &wrapped,
        &wrapped,
    );
}

#[test]
fn a_horizontal_inline_block_in_a_vertical_paragraph_is_no_wider_than_its_text() {
    // Three words are 290px, whatever the page offers.
    let words = ["wwwwwwwww"; 3].join(" ");
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p>{}</div>",
            horizontal_box("o", "", &words)
        ),
        &[
            ("p", (0.0, 0.0, 290.0, 600.0)),
            ("o", (0.0, 0.0, 290.0, 10.0)),
        ],
        &[
            ("p", (510.0, 0.0, 290.0, 600.0)),
            ("o", (510.0, 0.0, 290.0, 10.0)),
        ],
    );
}

#[test]
fn a_horizontal_inline_block_in_a_vertical_paragraph_leaves_room_for_its_margins() {
    // The 30px to each side come off the page, and the percentages of the
    // margins are of the 600px the lines run along: 10% is 60px.
    let sides = [
        ("p", (0.0, 0.0, 800.0, 600.0)),
        ("o", (30.0, 0.0, 740.0, 20.0)),
    ];
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p>{}</div>",
            horizontal_box("o", "margin:0 30px", &twelve_words())
        ),
        &sides,
        &sides,
    );
    let percent = [("o", (60.0, 0.0, 680.0, 20.0))];
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p>{}</div>",
            horizontal_box("o", "margin:0 10%", &twelve_words())
        ),
        &percent,
        &percent,
    );
    // The margins along the line take no room across it.
    let along = [("o", (0.0, 20.0, 800.0, 20.0))];
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p>{}</div>",
            horizontal_box("o", "margin:20px 0", &twelve_words())
        ),
        &along,
        &along,
    );
}

#[test]
fn a_horizontal_inline_block_in_a_vertical_paragraph_shares_its_line_with_text() {
    // The 20px of "aa" come before or after the box on the line.
    let before = [("o", (0.0, 20.0, 800.0, 20.0))];
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p>aa{}</div>",
            horizontal_box("o", "", &twelve_words())
        ),
        &before,
        &before,
    );
    let after = [("o", (0.0, 0.0, 800.0, 20.0))];
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p>{}aa</div>",
            horizontal_box("o", "", &twelve_words())
        ),
        &after,
        &after,
    );
}

#[test]
fn a_vertical_box_sized_by_its_content_is_as_tall_as_the_horizontal_box_it_holds() {
    // The height of a float, an inline-block or an absolutely positioned box
    // with a vertical writing mode follows its content: the two lines the
    // horizontal box wraps to at the page width, not the one it would take at
    // its max-content width.
    let tall = [
        ("p", (0.0, 0.0, 800.0, 20.0)),
        ("o", (0.0, 0.0, 800.0, 20.0)),
    ];
    for root in ["float:left", "display:inline-block", "position:absolute"] {
        check_modes(
            &format!(
                "{PAGE_MODE}<div id=p style=\"{root};writing-mode:MODE\">{}</div>",
                horizontal_box("o", "", &twelve_words())
            ),
            &tall,
            &tall,
        );
    }
    // A margin of 30px at each side narrows the box, not the height of the
    // two lines.
    let margins = [
        ("p", (0.0, 0.0, 800.0, 20.0)),
        ("o", (30.0, 0.0, 740.0, 20.0)),
    ];
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p style=\"float:left;writing-mode:MODE\">{}</div>",
            horizontal_box("o", "margin:0 30px", &twelve_words())
        ),
        &margins,
        &margins,
    );
}

#[test]
fn a_vertical_box_sized_by_its_content_stacks_the_horizontal_boxes_it_holds() {
    // The 20px of the first box and the 10px of the second add up, and the
    // narrower one is centred across the line.
    let five = ["wwwwwwwww"; 5].join(" ");
    let stacked = [
        ("p", (0.0, 0.0, 800.0, 30.0)),
        ("o", (0.0, 0.0, 800.0, 20.0)),
        ("q", (155.0, 20.0, 490.0, 10.0)),
    ];
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p style=\"float:left;writing-mode:MODE\">{}{}</div>",
            horizontal_box("o", "", &twelve_words()),
            horizontal_box("q", "", &five)
        ),
        &stacked,
        &stacked,
    );
}

#[test]
fn a_vertical_box_sized_by_its_content_is_as_tall_as_a_horizontal_box_with_a_width() {
    // The lines break at the width of the box, within its limits, and the
    // percentage is of the 800px page: 250px take two words of a line (six
    // lines), 300px three (four) and 400px four (three).
    let float = |css: &str| {
        format!(
            "{PAGE_MODE}<div id=p style=\"float:left;writing-mode:MODE\">{}</div>",
            horizontal_box("o", css, &twelve_words())
        )
    };
    check_modes(
        &float("width:250px"),
        &[
            ("p", (0.0, 0.0, 250.0, 60.0)),
            ("o", (0.0, 0.0, 250.0, 60.0)),
        ],
        &[
            ("p", (550.0, 0.0, 250.0, 60.0)),
            ("o", (550.0, 0.0, 250.0, 60.0)),
        ],
    );
    check_modes(
        &float("max-width:300px"),
        &[
            ("p", (0.0, 0.0, 300.0, 40.0)),
            ("o", (0.0, 0.0, 300.0, 40.0)),
        ],
        &[
            ("p", (500.0, 0.0, 300.0, 40.0)),
            ("o", (500.0, 0.0, 300.0, 40.0)),
        ],
    );
    check_modes(
        &float("width:50%"),
        &[
            ("p", (0.0, 0.0, 400.0, 30.0)),
            ("o", (0.0, 0.0, 400.0, 30.0)),
        ],
        &[
            ("p", (400.0, 0.0, 400.0, 30.0)),
            ("o", (400.0, 0.0, 400.0, 30.0)),
        ],
    );
    // The margins of 20px at each side add to the width of the root.
    check_modes(
        &float("width:250px;margin:0 20px"),
        &[
            ("p", (0.0, 0.0, 290.0, 60.0)),
            ("o", (20.0, 0.0, 250.0, 60.0)),
        ],
        &[
            ("p", (510.0, 0.0, 290.0, 60.0)),
            ("o", (530.0, 0.0, 250.0, 60.0)),
        ],
    );
}

#[test]
fn a_vertical_box_with_a_width_wraps_the_horizontal_box_it_holds_at_that_width() {
    // 300px fit three words of a line, so the twelve words take four lines.
    check_modes(
        &format!(
            "{PAGE_MODE}<div id=p style=\"float:left;width:300px;writing-mode:MODE\">{}</div>",
            horizontal_box("o", "", &twelve_words())
        ),
        &[
            ("p", (0.0, 0.0, 300.0, 40.0)),
            ("o", (0.0, 0.0, 300.0, 40.0)),
        ],
        &[
            ("p", (500.0, 0.0, 300.0, 40.0)),
            ("o", (500.0, 0.0, 300.0, 40.0)),
        ],
    );
}

#[test]
fn a_vertical_box_sized_by_its_content_is_as_tall_as_a_horizontal_block_it_holds() {
    // A block child of the vertical box is an orthogonal flow too: it fits
    // the page, less its margins, and the box is as tall as its lines.
    let block = |css: &str| {
        format!(
            "{PAGE_MODE}<div id=p style=\"float:left;writing-mode:MODE\">\
             <div id=i style=\"writing-mode:horizontal-tb;{css}\">{}</div></div>",
            twelve_words()
        )
    };
    let plain = [
        ("p", (0.0, 0.0, 800.0, 20.0)),
        ("i", (0.0, 0.0, 800.0, 20.0)),
    ];
    check_modes(&block(""), &plain, &plain);
    let margins = [
        ("p", (0.0, 0.0, 800.0, 20.0)),
        ("i", (40.0, 0.0, 720.0, 20.0)),
    ];
    check_modes(&block("margin:0 40px"), &margins, &margins);
}

#[test]
fn a_vertical_item_sized_by_its_content_is_as_tall_as_the_horizontal_box_it_holds() {
    // A flex item, a grid item, a table cell and a box with a keyword height
    // take the height of the two lines as well.
    let tall = [
        ("p", (0.0, 0.0, 800.0, 20.0)),
        ("o", (0.0, 0.0, 800.0, 20.0)),
    ];
    let held = horizontal_box("o", "", &twelve_words());
    let item = |outer: &str, css: &str, close: &str| {
        format!(
            "<style>body{{margin:0}}</style>{outer}\
             <div id=p style=\"writing-mode:MODE;{css}\">{held}</div>{close}"
        )
    };
    for (outer, css, close) in [
        ("<div style=\"display:flex\">", "", "</div>"),
        ("<div style=\"display:grid\">", "", "</div>"),
        (
            "<div style=\"display:table\"><div style=\"display:table-row\">",
            "display:table-cell",
            "</div></div>",
        ),
        ("", "height:min-content", ""),
        ("", "height:max-content", ""),
        ("", "height:fit-content", ""),
    ] {
        check_modes(&item(outer, css, close), &tall, &tall);
    }
}

#[test]
fn a_vertical_inline_block_in_a_horizontal_line_is_as_tall_as_the_horizontal_box_it_holds() {
    // The 20px of the two lines make the vertical box 20px tall, and the line
    // 32px.
    let line = [
        ("h", (0.0, 0.0, 800.0, 32.0)),
        ("p", (0.0, 10.0, 800.0, 20.0)),
        ("o", (0.0, 10.0, 800.0, 20.0)),
    ];
    check_modes(
        &format!(
            "<style>body{{margin:0}}</style><div id=h>aa<div id=p \
             style=\"display:inline-block;writing-mode:MODE\">{}</div></div>",
            horizontal_box("o", "", &twelve_words())
        ),
        &line,
        &line,
    );
}
