//! The min-content and max-content inline sizes of a multicol container
//! (CSS Multi-column 1 §3.4, CSS Sizing 3 §5): without `column-width` its
//! columns repeat the sizes of the content, `column-count` times with the
//! gaps between them; with `column-width` the min-content size is the
//! smaller of that width and the content's, and the max-content size is
//! that width for one column, or the larger of it and the content's for
//! each of the `column-count` columns. A vertical multicol container gives
//! these sizes along its height, which a vertical flex or grid container,
//! a vertical block that holds it and an intrinsic `height` read.
//!
//! The geometry below is what Chromium lays out for the same documents.

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

/// Thirteen words of four 10px glyphs each.
const WORDS: &str = "aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk llll mmmm";

/// The border box `(x, y, width, height)` of the element `id` in `body`.
fn rect(body: &str, id: &str) -> (f32, f32, f32, f32) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://multicol.test/document.html").expect("url"));
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
    let fragment = page
        .fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .unwrap_or_else(|| panic!("no #{id}"));
    let r = fragment.rect();
    (r.x, r.y, r.width, r.height)
}

fn assert_size(body: &str, id: &str, width: f32, height: f32) {
    let got = rect(body, id);
    assert!(
        (got.2 - width).abs() <= 0.02 && (got.3 - height).abs() <= 0.02,
        "#{id}: {got:?}, expected {width}x{height}"
    );
}

/// A vertical container `outer` 60px wide, in a 100px high box, that holds
/// a multicol container with `multicol` style.
fn vertical(outer: &str, multicol: &str, text: &str) -> String {
    format!(
        "<div style='height: 100px'><div id=o style='{outer}; width: 60px; \
         writing-mode: vertical-rl'><div id=i style='{multicol}'>{text}</div></div></div>"
    )
}

/// A vertical flex or grid container is as tall as the min-content height of
/// its multicol item when the room is smaller: three 40px columns and two
/// 10px gaps.
#[test]
fn a_vertical_flex_or_grid_container_grows_to_its_multicol_items_min_content() {
    for outer in [
        "display: flex",
        "display: inline-flex",
        "display: grid",
        "display: flex; flex-direction: column",
    ] {
        let body = vertical(outer, "column-count: 3", WORDS);
        assert_size(&body, "o", 60.0, 140.0);
    }
    let body = vertical("display: grid", "column-count: 3; float: left", WORDS);
    assert_size(&body, "o", 60.0, 140.0);
    let body = format!(
        "<div style='height: 100px'><div id=o style='display: flex; width: 60px; \
         writing-mode: vertical-lr'><div id=i style='column-count: 3'>{WORDS}</div></div></div>"
    );
    assert_size(&body, "o", 60.0, 140.0);
    assert_size(&body, "i", 60.0, 140.0);
}

/// The gaps, the padding and the longest word of the multicol item count;
/// a later item follows the multicol one.
#[test]
fn the_gaps_padding_and_words_of_the_columns_decide_the_height() {
    let gap = vertical("display: flex", "column-count: 3; column-gap: 5px", WORDS);
    assert_size(&gap, "o", 60.0, 130.0);
    let padding = vertical("display: flex", "column-count: 3; padding: 5px", WORDS);
    assert_size(&padding, "o", 60.0, 150.0);
    let long = vertical("display: flex", "column-count: 3", "aaaaaaaaaaaaaaa bbbb");
    assert_size(&long, "o", 60.0, 470.0);
    let two_items = format!(
        "<div style='height: 100px'><div id=o style='display: flex; width: 60px; \
         writing-mode: vertical-rl'><div id=i style='column-count: 3'>{WORDS}</div>\
         <div id=j>xx</div></div></div>"
    );
    assert_size(&two_items, "o", 60.0, 160.0);
    assert_size(&two_items, "i", 60.0, 140.0);
    assert_eq!(rect(&two_items, "j").1, 140.0);
}

/// Columns that fit the room, or are narrower than it, leave the container
/// at the room.
#[test]
fn a_multicol_item_that_fits_the_room_takes_the_room() {
    let fits = vertical("display: flex", "column-count: 2", WORDS);
    assert_size(&fits, "o", 60.0, 100.0);
    let short = vertical("display: flex", "column-count: 3", "aa bb");
    assert_size(&short, "o", 60.0, 100.0);
    let taller = vertical("display: flex", "column-count: 3", WORDS).replace("100px", "200px");
    assert_size(&taller, "o", 60.0, 200.0);
}

/// A vertical block shrinks its height to fit the multicol box it holds, and
/// a `height: min-content` takes the min-content height of the columns.
#[test]
fn a_vertical_block_fits_its_height_to_the_multicol_box_it_holds() {
    let wide = vertical("display: block", "column-count: 3", WORDS);
    assert_size(&wide, "o", 60.0, 140.0);
    let short = vertical("display: block", "column-count: 3", "aa bb");
    assert_size(&short, "o", 60.0, 100.0);
    let min_content = format!(
        "<div id=o style='writing-mode: vertical-rl; height: min-content'>\
         <div id=i style='column-count: 3'>{WORDS}</div></div>"
    );
    assert_eq!(rect(&min_content, "o").3, 140.0);
    let max_content = "<div id=o style='writing-mode: vertical-rl; height: max-content'>\
         <div id=i style='column-count: 3'>aa bb</div></div>";
    assert_eq!(rect(max_content, "o").3, 170.0);
    // Between lines of text of the vertical block, too.
    let between_lines = format!(
        "<div style='height: 100px'><div id=o style='width: 60px; writing-mode: vertical-rl'>\
         xx<div id=i style='column-count: 3'>{WORDS}</div>yy</div></div>"
    );
    assert_size(&between_lines, "o", 60.0, 140.0);
}

/// With both `column-width` and `column-count`, the min-content width is the
/// smaller of the column width and the longest word, and the max-content
/// width repeats the larger of the column width and the content's width.
/// The count is cut to the columns of at least the column width that fit,
/// so a container no wider than one column has a single column.
#[test]
fn a_column_width_with_a_count_sizes_and_counts_columns_like_chromium() {
    let cases = [
        // (column-width, column-count, text, min-content, max-content)
        (30, "1", "aa bb", 20.0, 50.0),
        (30, "2", "aa bb", 20.0, 110.0),
        (70, "2", "aa bb", 20.0, 150.0),
        (30, "3", "aaaaaaaaaa b", 30.0, 380.0),
        (50, "2", WORDS, 40.0, 1290.0),
        (70, "3", "aaaaaaaaaa b", 70.0, 380.0),
    ];
    for (column_width, count, text, min, max) in cases {
        let style = format!("column-width: {column_width}px; column-count: {count}");
        let min_body =
            format!("<div id=o style='width: min-content'><div style='{style}'>{text}</div></div>");
        assert_eq!(
            rect(&min_body, "o").2,
            min,
            "min-content of {style} / {text}"
        );
        let max_body =
            format!("<div id=o style='width: max-content'><div style='{style}'>{text}</div></div>");
        assert_eq!(
            rect(&max_body, "o").2,
            max,
            "max-content of {style} / {text}"
        );
    }
    // One 30px column holds the thirteen words on thirteen lines.
    let narrow = format!(
        "<div id=o style='width: min-content'><div id=i style='column-width: 30px; \
         column-count: 2'>{WORDS}</div></div>"
    );
    assert_size(&narrow, "i", 30.0, 130.0);
}
