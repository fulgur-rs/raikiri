//! A column of a horizontal grid takes the width of a vertical item as the
//! lines of the item are laid out along the rows: the item is an orthogonal
//! flow, and the room its lines fit is the height of the grid when that is
//! definite, else no limit, as the rows grow to the lines (CSS Writing Modes 4
//! §7.3.1, CSS Grid 1 §11.5). The text `AAAA BBBB` in Ahem at 10px stays on
//! one line of 90px in an open height, wraps into two columns of 10px in a
//! height of 70px, and the longest word is 40px. The expected border boxes are
//! the ones Chromium lays out for the same markup, in an 800px by 600px
//! viewport with a body margin of 8px.

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

/// Twenty words of 100px with a space of 10px between them: 2190px of text,
/// three words (320px) to a line of 330px and two to a line of 310px.
fn twenty_words() -> String {
    ["AAAAAAAAAA"; 20].join(" ")
}

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page with an 8px margin.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
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

/// A grid of `grid_css` in a 400px flex container, holding the vertical item
/// `#i` of `item_css` with `text`.
fn flex_grid(grid_css: &str, item_css: &str, text: &str) -> String {
    format!(
        "<div style=\"display:flex;width:400px\"><div id=g style=\"display:grid;{grid_css}\">\
         <div id=i style=\"writing-mode:vertical-rl;{item_css}\">{text}</div></div></div>"
    )
}

#[test]
fn a_grid_of_open_height_fits_the_lines_of_a_vertical_item_in_one_column() {
    // Neither the smallest height of the item nor the page decides the lines:
    // the text stays on its line of 90px, one column wide, whichever way the
    // width of the grid asks for the item.
    for (grid_css, height) in [
        ("width:min-content", 90.0),
        ("width:max-content", 90.0),
        ("width:fit-content", 90.0),
        ("", 90.0),
        ("width:min-content;grid-template-rows:auto", 90.0),
        // A maximum height does not make the height definite; the minimum
        // and the stretched rows only grow the grid.
        ("width:min-content;max-height:200px", 90.0),
        ("width:min-content;min-height:100px", 100.0),
        (
            "width:min-content;height:100px;grid-template-rows:auto",
            100.0,
        ),
    ] {
        assert_boxes(
            &flex_grid(grid_css, "", TEXT),
            &[
                ("g", [8.0, 8.0, 10.0, height]),
                ("i", [8.0, 8.0, 10.0, height]),
            ],
        );
    }
    assert_boxes(
        &flex_grid("width:min-content", "writing-mode:vertical-lr", TEXT),
        &[("g", [8.0, 8.0, 10.0, 90.0]), ("i", [8.0, 8.0, 10.0, 90.0])],
    );
}

#[test]
fn an_inline_grid_and_a_column_sized_by_content_fit_the_lines_in_one_column() {
    for grid_css in [
        "display:inline-grid",
        "display:inline-grid;grid-template-columns:min-content",
    ] {
        assert_boxes(
            &format!(
                "<div><div id=g style=\"{grid_css}\"><div id=i style=\"writing-mode:vertical-rl\">\
                 {TEXT}</div></div></div>"
            ),
            &[("g", [8.0, 8.0, 10.0, 90.0]), ("i", [8.0, 8.0, 10.0, 90.0])],
        );
    }
    // The track of a grid as wide as the page is sized by the item as well.
    assert_boxes(
        &format!(
            "<div id=g style=\"display:grid;grid-template-columns:min-content\">\
             <div id=i style=\"writing-mode:vertical-rl\">{TEXT}</div></div>"
        ),
        &[
            ("g", [8.0, 8.0, 784.0, 90.0]),
            ("i", [8.0, 8.0, 10.0, 90.0]),
        ],
    );
}

#[test]
fn the_columns_of_a_grid_take_the_width_of_each_of_their_items() {
    // The horizontal item is 40px wide at its longest word, the vertical one a
    // line of 10px.
    assert_boxes(
        &format!(
            "<div style=\"display:flex;width:400px\"><div id=g style=\"display:grid;\
             width:min-content;grid-template-columns:auto auto\">\
             <div id=i style=\"writing-mode:vertical-rl\">{TEXT}</div><div id=j>{TEXT}</div>\
             </div></div>"
        ),
        &[
            ("g", [8.0, 8.0, 50.0, 90.0]),
            ("i", [8.0, 8.0, 10.0, 90.0]),
            ("j", [18.0, 8.0, 40.0, 90.0]),
        ],
    );
    assert_boxes(
        &format!(
            "<div style=\"display:flex;width:400px\"><div id=g style=\"display:grid;\
             width:min-content;grid-template-columns:auto auto\">\
             <div id=i style=\"writing-mode:vertical-lr\">{TEXT}</div>\
             <div id=j style=\"writing-mode:vertical-rl\">{TEXT}</div></div></div>"
        ),
        &[
            ("g", [8.0, 8.0, 20.0, 90.0]),
            ("i", [8.0, 8.0, 10.0, 90.0]),
            ("j", [18.0, 8.0, 10.0, 90.0]),
        ],
    );
    // Two rows hold the item once each, and an item over two columns spans
    // them as one.
    assert_boxes(
        &format!(
            "<div style=\"display:flex;width:400px\"><div id=g style=\"display:grid;\
             width:min-content;grid-template-rows:auto auto\">\
             <div id=i style=\"writing-mode:vertical-rl\">{TEXT}</div>\
             <div id=j style=\"writing-mode:vertical-rl\">{TEXT}</div></div></div>"
        ),
        &[
            ("g", [8.0, 8.0, 10.0, 180.0]),
            ("i", [8.0, 8.0, 10.0, 90.0]),
            ("j", [8.0, 98.0, 10.0, 90.0]),
        ],
    );
    assert_boxes(
        &format!(
            "<div style=\"display:flex;width:400px\"><div id=g style=\"display:grid;\
             width:min-content;grid-template-columns:auto auto\">\
             <div id=i style=\"writing-mode:vertical-rl;grid-column:span 2\">{TEXT}</div>\
             </div></div>"
        ),
        &[("g", [8.0, 8.0, 10.0, 90.0]), ("i", [8.0, 8.0, 10.0, 90.0])],
    );
}

#[test]
fn a_vertical_block_container_item_takes_its_columns_the_same_way() {
    assert_boxes(
        &format!(
            "<div style=\"display:flex;width:400px\"><div id=g style=\"display:grid;\
             width:min-content\"><div id=i style=\"writing-mode:vertical-rl\">\
             <div id=c>{TEXT}</div><div id=d>CC</div></div></div></div>"
        ),
        &[
            ("g", [8.0, 8.0, 20.0, 90.0]),
            ("i", [8.0, 8.0, 20.0, 90.0]),
            ("c", [18.0, 8.0, 10.0, 90.0]),
            ("d", [8.0, 8.0, 10.0, 90.0]),
        ],
    );
}

#[test]
fn a_definite_height_of_the_grid_is_the_room_the_lines_of_the_item_fit() {
    for (grid_css, width, height) in [
        // The 90px of text fit 100px, and break in two columns of 70px.
        ("height:100px", 10.0, 100.0),
        ("height:70px", 20.0, 70.0),
        // A height below the smallest of the item keeps its longest word.
        ("height:30px", 20.0, 30.0),
        ("height:70px;grid-template-rows:auto", 20.0, 70.0),
        // A row of its own length is the room of its item too.
        ("grid-template-rows:60px", 20.0, 60.0),
        ("grid-template-rows:100px", 10.0, 100.0),
        // The height is the one of the border box, less the padding.
        ("height:70px;padding:10px 0", 20.0, 70.0),
        ("height:110px;padding:10px 0", 10.0, 110.0),
    ] {
        assert_boxes(
            &flex_grid(&format!("width:min-content;{grid_css}"), "", TEXT),
            &[("g", [8.0, 8.0, width, height])],
        );
    }
    // The height of the row is what the item takes of it: 40px for its
    // longest word, in a grid of 30px.
    assert_boxes(
        &flex_grid("width:min-content;height:30px", "", TEXT),
        &[("i", [8.0, 8.0, 20.0, 40.0])],
    );
}

#[test]
fn a_height_that_a_container_makes_definite_is_the_room_of_the_item() {
    let text = twenty_words();
    // A percentage height of a container of 300px.
    assert_boxes(
        &format!(
            "<div style=\"height:300px\"><div id=g style=\"display:grid;width:min-content;\
             height:100%\"><div id=i style=\"writing-mode:vertical-rl\">{text}</div></div></div>"
        ),
        &[
            ("g", [8.0, 8.0, 100.0, 300.0]),
            ("i", [8.0, 8.0, 100.0, 300.0]),
        ],
    );
    // A grid that a flex container of 300px stretches.
    assert_boxes(
        &format!(
            "<div style=\"display:flex;height:300px\"><div id=g style=\"display:grid\">\
             <div id=i style=\"writing-mode:vertical-rl\">{text}</div></div></div>"
        ),
        &[
            ("g", [8.0, 8.0, 100.0, 300.0]),
            ("i", [8.0, 8.0, 100.0, 300.0]),
        ],
    );
}

#[test]
fn the_margins_and_padding_come_off_the_room_of_the_item() {
    let text = twenty_words();
    // 330px hold three words to a line, seven lines of 10px.
    assert_boxes(
        &flex_grid("width:min-content;height:330px", "", &text),
        &[
            ("g", [8.0, 8.0, 70.0, 330.0]),
            ("i", [8.0, 8.0, 70.0, 330.0]),
        ],
    );
    // 10px of margin to each side leave 310px: two words, ten lines.
    assert_boxes(
        &flex_grid("width:min-content;height:330px", "margin:10px 0", &text),
        &[
            ("g", [8.0, 8.0, 100.0, 330.0]),
            ("i", [8.0, 18.0, 100.0, 310.0]),
        ],
    );
    assert_boxes(
        &flex_grid(
            "width:min-content;grid-template-rows:330px",
            "margin:10px 0",
            &text,
        ),
        &[
            ("g", [8.0, 8.0, 100.0, 330.0]),
            ("i", [8.0, 18.0, 100.0, 310.0]),
        ],
    );
    // The same 20px as padding of the grid.
    assert_boxes(
        &flex_grid("width:min-content;height:330px;padding:10px 0", "", &text),
        &[
            ("g", [8.0, 8.0, 100.0, 330.0]),
            ("i", [8.0, 18.0, 100.0, 310.0]),
        ],
    );
    // The margin of an item in an open height only moves it.
    assert_boxes(
        &flex_grid("width:min-content", "margin:5px 0", TEXT),
        &[
            ("g", [8.0, 8.0, 10.0, 100.0]),
            ("i", [8.0, 13.0, 10.0, 90.0]),
        ],
    );
}
