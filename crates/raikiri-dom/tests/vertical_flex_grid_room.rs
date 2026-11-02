//! The height a vertical flex container fits its lines in, and the height a
//! grid hands to its vertical items, in a horizontal flow.
//!
//! The inline size of a vertical box is its height. An auto one is the
//! fit-content size: the room offered, but at least the min-content and at most
//! the max-content height, or the height of the grid row the box is stretched
//! to (CSS Sizing 3 §3.1.1, CSS Writing Modes 4 §7.3.1). A grid whose height is
//! open but limited by its `max-height` grows its rows up to that limit, and
//! its vertical items fit their lines in it (CSS Grid 2 §11.7). The text
//! `AAAA BBBB` in Ahem at 10px stays on one line of 90px, wraps into two
//! columns of 10px in a height of 70px, and the longest word is 40px. The
//! expected border boxes are the ones Chromium lays out for the same markup, in
//! an 800px by 600px viewport without a body margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

const TEXT: &str = "AAAA BBBB";

/// A hundred words of `aa`: 3000px of text on one line, ten columns in a height
/// of 300px.
fn hundred_words() -> String {
    ["aa"; 100].join(" ")
}

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page without margins.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
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
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Check that every `(id, expected box)` of `expected` is laid out as given.
fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    let actual = boxes(body);
    for &(id, want) in expected {
        let got = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < 0.01),
            "#{id} is {got:?}, expected {want:?} for {body}"
        );
    }
}

/// A grid of `grid_css` in a 400px flex container, holding the vertical flex
/// container `#i` of `item_css` with `TEXT`.
fn flex_item_grid(grid_css: &str, item_css: &str) -> String {
    format!(
        "<div style=\"display:flex;width:400px\"><div id=g style=\"display:grid;{grid_css}\">\
         <div id=i style=\"writing-mode:vertical-rl;display:flex;{item_css}\">{TEXT}</div></div></div>"
    )
}

#[test]
fn a_vertical_flex_item_of_a_grid_is_stretched_to_the_row_and_fits_its_lines_in_it() {
    // The grid is 70px tall, so the lines of the item break into two columns
    // of 10px at 70px, and the column takes the width of both. In 100px they
    // stay on one line.
    for (grid_css, item_css, grid, item) in [
        (
            "width:min-content;height:70px",
            "",
            [0.0, 0.0, 20.0, 70.0],
            [0.0, 0.0, 20.0, 70.0],
        ),
        (
            "width:min-content;height:100px",
            "",
            [0.0, 0.0, 10.0, 100.0],
            [0.0, 0.0, 10.0, 100.0],
        ),
        // An open height gives the lines no limit.
        (
            "width:min-content",
            "",
            [0.0, 0.0, 10.0, 90.0],
            [0.0, 0.0, 10.0, 90.0],
        ),
        (
            "width:max-content",
            "",
            [0.0, 0.0, 10.0, 90.0],
            [0.0, 0.0, 10.0, 90.0],
        ),
        ("", "", [0.0, 0.0, 10.0, 90.0], [0.0, 0.0, 10.0, 90.0]),
        (
            "width:min-content",
            "writing-mode:vertical-lr",
            [0.0, 0.0, 10.0, 90.0],
            [0.0, 0.0, 10.0, 90.0],
        ),
        (
            "width:min-content",
            "display:inline-flex",
            [0.0, 0.0, 10.0, 90.0],
            [0.0, 0.0, 10.0, 90.0],
        ),
        // The padding and the margins come off the height the lines fit.
        (
            "width:min-content",
            "padding:5px",
            [0.0, 0.0, 20.0, 100.0],
            [0.0, 0.0, 20.0, 100.0],
        ),
        (
            "width:min-content",
            "margin:5px 0",
            [0.0, 0.0, 10.0, 100.0],
            [0.0, 5.0, 10.0, 90.0],
        ),
        // The limits of the item bound the height it is stretched to.
        (
            "width:min-content;height:70px",
            "max-height:50px",
            [0.0, 0.0, 20.0, 70.0],
            [0.0, 0.0, 20.0, 50.0],
        ),
        (
            "width:min-content;height:70px",
            "min-height:80px",
            [0.0, 0.0, 20.0, 70.0],
            [0.0, 0.0, 20.0, 80.0],
        ),
        (
            "width:min-content;height:70px",
            "height:40px",
            [0.0, 0.0, 20.0, 70.0],
            [0.0, 0.0, 20.0, 40.0],
        ),
        // A grid whose maximum height limits it, and whose height is open,
        // sizes its column by the lines in one column; the rows then grow
        // to the limit.
        (
            "width:min-content;max-height:70px",
            "",
            [0.0, 0.0, 10.0, 70.0],
            [0.0, 0.0, 20.0, 70.0],
        ),
    ] {
        assert_boxes(
            &flex_item_grid(grid_css, item_css),
            &[("g", grid), ("i", item)],
        );
    }
}

#[test]
fn a_vertical_flex_item_of_a_grid_that_is_not_stretched_fits_the_height_of_its_row() {
    // The item is as tall as the row allows, but no taller than its lines.
    for (grid_css, item_css, grid, item) in [
        (
            "width:min-content;height:70px",
            "align-self:start",
            [0.0, 0.0, 20.0, 70.0],
            [0.0, 0.0, 20.0, 70.0],
        ),
        (
            "width:min-content;height:70px",
            "align-self:center",
            [0.0, 0.0, 20.0, 70.0],
            [0.0, 0.0, 20.0, 70.0],
        ),
        (
            "width:min-content;height:70px",
            "align-self:end",
            [0.0, 0.0, 20.0, 70.0],
            [0.0, 0.0, 20.0, 70.0],
        ),
        (
            "width:min-content;height:70px",
            "margin:auto 0",
            [0.0, 0.0, 20.0, 70.0],
            [0.0, 0.0, 20.0, 70.0],
        ),
        (
            "width:min-content;height:100px",
            "align-self:start",
            [0.0, 0.0, 10.0, 100.0],
            [0.0, 0.0, 10.0, 90.0],
        ),
        (
            "width:min-content;height:70px",
            "flex-direction:column",
            [0.0, 0.0, 20.0, 70.0],
            [0.0, 0.0, 20.0, 70.0],
        ),
    ] {
        assert_boxes(
            &flex_item_grid(grid_css, item_css),
            &[("g", grid), ("i", item)],
        );
    }
}

/// A vertical flex container `#i` with the hundred words in `parent` (a
/// markup with `{}` where the container goes), laid out in an 800px by 600px
/// page.
fn flex_in(parent: &str, item_css: &str) -> String {
    parent.replace(
        "{}",
        &format!(
            "<div id=i style=\"writing-mode:vertical-rl;display:flex;{item_css}\">{}</div>",
            hundred_words()
        ),
    )
}

#[test]
fn a_vertical_flex_container_fits_its_lines_in_the_height_its_parent_leaves() {
    // With no height of the parent in the way, the lines break at the page, in
    // five columns of 10px; a container with a limit of its own breaks them
    // at that, in ten columns, and a definite height is the room as well.
    for (parent, item_css, item) in [
        (
            "<div style=\"width:400px\">{}</div>",
            "",
            [0.0, 0.0, 50.0, 600.0],
        ),
        (
            "<div style=\"display:flex;width:400px\">{}</div>",
            "",
            [0.0, 0.0, 50.0, 600.0],
        ),
        (
            "<div style=\"width:400px\">{}</div>",
            "float:left",
            [0.0, 0.0, 50.0, 600.0],
        ),
        (
            "<div style=\"width:400px\">{}</div>",
            "display:inline-flex",
            [0.0, 0.0, 50.0, 600.0],
        ),
        (
            "<div style=\"width:400px;max-height:300px\">{}</div>",
            "",
            [0.0, 0.0, 100.0, 300.0],
        ),
        (
            "<div style=\"display:flex;width:400px;max-height:300px\">{}</div>",
            "",
            [0.0, 0.0, 100.0, 300.0],
        ),
        (
            "<div style=\"width:400px;max-height:300px\">{}</div>",
            "float:left",
            [0.0, 0.0, 100.0, 300.0],
        ),
        (
            "<div style=\"width:400px;max-height:300px\">{}</div>",
            "display:inline-flex",
            [0.0, 0.0, 100.0, 300.0],
        ),
        (
            "<div style=\"width:400px;height:200px\">{}</div>",
            "",
            [0.0, 0.0, 150.0, 200.0],
        ),
    ] {
        assert_boxes(&flex_in(parent, item_css), &[("i", item)]);
    }
}

/// A grid of `grid_css`, 300px wide, whose items `#o` and `#q` of `item_css`
/// hold the hundred words each (`#q` when `two`).
fn max_height_grid(grid_css: &str, item_css: &str, two: bool) -> String {
    let item = |id: &str| {
        format!(
            "<div id={id} style=\"writing-mode:vertical-rl;justify-self:start;{item_css}\">{}</div>",
            hundred_words()
        )
    };
    format!(
        "<div id=p style=\"display:grid;width:300px;{grid_css}\">{}{}</div>",
        item("o"),
        if two { item("q") } else { String::new() }
    )
}

#[test]
fn a_grid_of_open_height_fits_its_vertical_items_in_what_its_max_height_leaves() {
    // The rows grow up to the limit: ten columns of 10px in 300px. A border
    // box limit loses the padding, a content box limit keeps it, and the
    // margins of the item come off. A minimum height raises the limit; a
    // percentage of an open height is no limit, and nor is a minimum alone.
    for (grid_css, item_css, grid, item) in [
        (
            "max-height:300px",
            "",
            [0.0, 0.0, 300.0, 300.0],
            [0.0, 0.0, 100.0, 300.0],
        ),
        (
            "max-height:calc(100px + 200px)",
            "",
            [0.0, 0.0, 300.0, 300.0],
            [0.0, 0.0, 100.0, 300.0],
        ),
        (
            "max-height:300px",
            "align-self:start",
            [0.0, 0.0, 300.0, 300.0],
            [0.0, 0.0, 100.0, 300.0],
        ),
        (
            "max-height:300px",
            "display:flex",
            [0.0, 0.0, 300.0, 300.0],
            [0.0, 0.0, 100.0, 300.0],
        ),
        (
            "max-height:300px",
            "margin:5px 0",
            [0.0, 0.0, 300.0, 300.0],
            [0.0, 5.0, 100.0, 290.0],
        ),
        (
            "max-height:300px;padding:10px 0",
            "",
            [0.0, 0.0, 300.0, 300.0],
            [0.0, 10.0, 120.0, 280.0],
        ),
        (
            "max-height:300px;padding:10px 0;box-sizing:content-box",
            "",
            [0.0, 0.0, 300.0, 320.0],
            [0.0, 10.0, 100.0, 300.0],
        ),
        (
            "max-height:300px;padding:10% 0",
            "",
            [0.0, 0.0, 300.0, 300.0],
            [0.0, 80.0, 200.0, 140.0],
        ),
        (
            "max-height:300px;padding:10% 0;box-sizing:content-box",
            "",
            [0.0, 0.0, 300.0, 460.0],
            [0.0, 80.0, 100.0, 300.0],
        ),
        (
            "min-height:400px;max-height:300px",
            "",
            [0.0, 0.0, 300.0, 400.0],
            [0.0, 0.0, 80.0, 400.0],
        ),
        // A width of min-content does not see the limit; the item overflows
        // the column it asked for.
        (
            "max-height:300px;width:min-content",
            "",
            [0.0, 0.0, 10.0, 300.0],
            [0.0, 0.0, 100.0, 300.0],
        ),
    ] {
        assert_boxes(
            &max_height_grid(grid_css, item_css, false),
            &[("p", grid), ("o", item)],
        );
    }
    // The rows of two items share the limit.
    assert_boxes(
        &max_height_grid("max-height:300px", "", true),
        &[
            ("p", [0.0, 0.0, 300.0, 300.0]),
            ("o", [0.0, 0.0, 200.0, 150.0]),
            ("q", [0.0, 150.0, 200.0, 150.0]),
        ],
    );
}

#[test]
fn a_short_text_keeps_the_height_of_its_line_under_a_grid_max_height() {
    // The text fits 300px on one line, so the grid is as tall as the line.
    assert_boxes(
        &format!(
            "<div id=p style=\"display:grid;width:300px;max-height:300px\">\
             <div id=o style=\"writing-mode:vertical-rl;justify-self:start\">{TEXT}</div></div>"
        ),
        &[
            ("p", [0.0, 0.0, 300.0, 90.0]),
            ("o", [0.0, 0.0, 10.0, 90.0]),
        ],
    );
}

#[test]
fn a_flex_item_stretched_to_a_max_height_a_percentage_padding_drops_is_no_room() {
    // The item is stretched to 300px by its border-box limit. A padding in
    // pixels comes off it, and the box fits its lines in the 220px that are
    // left, fifteen columns. A padding in percent makes a browser drop the
    // limit, and the stretched height is no room either: the lines fit the page
    // area, in five columns, and the box is cut by the page.
    let flex = |padding: &str| {
        format!(
            "<div style=\"display:flex;width:400px\"><div id=p style=\"width:300px;\
             max-height:300px;padding:{padding}\"><div id=o style=\"writing-mode:vertical-rl\">{}\
             </div></div></div>",
            hundred_words()
        )
    };
    assert_boxes(
        &flex("40px 0"),
        &[
            ("p", [0.0, 0.0, 300.0, 300.0]),
            ("o", [0.0, 40.0, 150.0, 220.0]),
        ],
    );
    // A flex container with a height of its own makes the stretch definite,
    // whatever the limit comes to.
    for container_height in [300, 500] {
        assert_boxes(
            &flex("10% 0").replace(
                "display:flex;width:400px",
                &format!("display:flex;width:400px;height:{container_height}px"),
            ),
            &[
                ("p", [0.0, 0.0, 300.0, 300.0]),
                ("o", [0.0, 40.0, 150.0, 220.0]),
            ],
        );
    }
    let boxes = boxes(&flex("10% 0"));
    let width = |id: &str| boxes.iter().find(|(name, _)| name == id).map(|(_, b)| b[2]);
    assert_eq!(width("p"), Some(300.0));
    assert_eq!(width("o"), Some(50.0));
}

#[test]
fn a_max_height_does_not_shrink_the_row_of_a_horizontal_item() {
    // Only the lines of a vertical item see the limit: the row of a horizontal
    // item that may shrink to nothing keeps the height of its content, and the
    // grid is cut to the limit.
    assert_boxes(
        "<div id=p style=\"display:grid;width:100px;max-height:10px\">\
         <div id=o style=\"min-height:0;font:100px/1 Ahem\">X</div></div>",
        &[
            ("p", [0.0, 0.0, 100.0, 10.0]),
            ("o", [0.0, 0.0, 100.0, 100.0]),
        ],
    );
}
