//! Lines of text around a column spanner in a multicol container with a
//! definite or a maximum height. The lines above the spanner balance in their
//! columns, and those below it fill what the height leaves; past it they go
//! on in more columns (CSS Multi-column 1, 7.1 and 8.1).
//!
//! Each case is a document and the border box of each element with an id, as
//! Chromium lays it out in an 800x600 viewport with a body margin of 8px and
//! 10px Ahem text.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The border box of every element with an id in `body`, by id.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:8px;font:10px/10px Ahem}}div{{box-sizing:border-box}}</style>{body}"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let page = result.page(0).unwrap();
    let dom = page.dom();
    let mut boxes: Vec<_> = page
        .fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect();
    boxes.sort_by(|a, b| a.0.cmp(&b.0));
    boxes
}

fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    let boxes = boxes(body);
    for (id, rect) in expected {
        let found = boxes.iter().find(|(found, _)| found == id);
        assert_eq!(found.map(|(_, rect)| *rect), Some(*rect), "{id}");
    }
}

#[test]
fn a_definite_height_balances_the_lines_above_and_below_a_spanner() {
    assert_boxes(
        "<div id=m style='orphans:1;widows:1;columns:2;column-gap:0;width:200px;height:100px'><span id=a>A</span><br><span id=b>B</span><div id=s style='column-span:all;height:10px'></div><span id=c>C</span><br><span id=d>D</span></div>",
        &[
            ("m", [8.0, 8.0, 200.0, 100.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("b", [108.0, 8.0, 10.0, 10.0]),
            ("s", [8.0, 18.0, 200.0, 10.0]),
            ("c", [8.0, 28.0, 10.0, 10.0]),
            ("d", [108.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn the_last_group_of_a_definite_height_fills_its_columns_in_turn() {
    assert_boxes(
        "<div id=m style='orphans:1;widows:1;columns:2;column-gap:0;width:200px;height:100px;column-fill:auto'><span id=a>A</span><br><span id=b>B</span><div id=s style='column-span:all;height:10px'></div><span id=c>C</span><br><span id=d>D</span></div>",
        &[
            ("m", [8.0, 8.0, 200.0, 100.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("b", [108.0, 8.0, 10.0, 10.0]),
            ("s", [8.0, 18.0, 200.0, 10.0]),
            ("c", [8.0, 28.0, 10.0, 10.0]),
            ("d", [8.0, 38.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn the_last_group_overflows_into_more_columns_past_a_definite_height() {
    assert_boxes(
        "<div id=m style='orphans:1;widows:1;columns:2;column-gap:0;width:200px;height:40px'><span id=a>A</span><br><span id=b>B</span><div id=s style='column-span:all;height:10px'></div><span id=l0>L</span><br><span id=l1>L</span><br><span id=l2>L</span><br><span id=l3>L</span><br><span id=l4>L</span><br><span id=l5>L</span><br><span id=l6>L</span><br><span id=l7>L</span><br><span id=l8>L</span><br><span id=l9>L</span><br></div>",
        &[
            ("m", [8.0, 8.0, 200.0, 40.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("b", [108.0, 8.0, 10.0, 10.0]),
            ("s", [8.0, 18.0, 200.0, 10.0]),
            ("l0", [8.0, 28.0, 10.0, 10.0]),
            ("l1", [8.0, 38.0, 10.0, 10.0]),
            ("l2", [108.0, 28.0, 10.0, 10.0]),
            ("l3", [108.0, 38.0, 10.0, 10.0]),
            ("l4", [208.0, 28.0, 10.0, 10.0]),
            ("l5", [208.0, 38.0, 10.0, 10.0]),
            ("l6", [308.0, 28.0, 10.0, 10.0]),
            ("l7", [308.0, 38.0, 10.0, 10.0]),
            ("l8", [408.0, 28.0, 10.0, 10.0]),
            ("l9", [408.0, 38.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn the_last_group_filled_in_turn_overflows_past_a_definite_height() {
    assert_boxes(
        "<div id=m style='orphans:1;widows:1;columns:2;column-gap:0;width:200px;height:40px;column-fill:auto'><span id=a>A</span><br><span id=b>B</span><div id=s style='column-span:all;height:10px'></div><span id=l0>L</span><br><span id=l1>L</span><br><span id=l2>L</span><br><span id=l3>L</span><br><span id=l4>L</span><br><span id=l5>L</span><br><span id=l6>L</span><br><span id=l7>L</span><br><span id=l8>L</span><br><span id=l9>L</span><br></div>",
        &[
            ("m", [8.0, 8.0, 200.0, 40.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("b", [108.0, 8.0, 10.0, 10.0]),
            ("s", [8.0, 18.0, 200.0, 10.0]),
            ("l0", [8.0, 28.0, 10.0, 10.0]),
            ("l1", [8.0, 38.0, 10.0, 10.0]),
            ("l2", [108.0, 28.0, 10.0, 10.0]),
            ("l3", [108.0, 38.0, 10.0, 10.0]),
            ("l4", [208.0, 28.0, 10.0, 10.0]),
            ("l5", [208.0, 38.0, 10.0, 10.0]),
            ("l6", [308.0, 28.0, 10.0, 10.0]),
            ("l7", [308.0, 38.0, 10.0, 10.0]),
            ("l8", [408.0, 28.0, 10.0, 10.0]),
            ("l9", [408.0, 38.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn the_padding_of_a_definite_height_container_is_left_out_of_its_columns() {
    assert_boxes(
        "<div id=m style='orphans:1;widows:1;columns:2;column-gap:0;width:200px;height:60px;padding:5px;box-sizing:border-box'><span id=a>A</span><br><span id=b>B</span><div id=s style='column-span:all;height:10px'></div><span id=l0>L</span><br><span id=l1>L</span><br><span id=l2>L</span><br><span id=l3>L</span><br><span id=l4>L</span><br><span id=l5>L</span><br></div>",
        &[
            ("m", [8.0, 8.0, 200.0, 60.0]),
            ("a", [13.0, 13.0, 10.0, 10.0]),
            ("b", [108.0, 13.0, 10.0, 10.0]),
            ("s", [13.0, 23.0, 190.0, 10.0]),
            ("l0", [13.0, 33.0, 10.0, 10.0]),
            ("l1", [13.0, 43.0, 10.0, 10.0]),
            ("l2", [13.0, 53.0, 10.0, 10.0]),
            ("l3", [108.0, 33.0, 10.0, 10.0]),
            ("l4", [108.0, 43.0, 10.0, 10.0]),
            ("l5", [108.0, 53.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn a_definite_height_the_spanner_fills_leaves_a_line_in_each_column() {
    assert_boxes(
        "<div id=m style='orphans:1;widows:1;columns:2;column-gap:0;width:200px;height:20px'><span id=a>A</span><br><span id=b>B</span><div id=s style='column-span:all;height:10px'></div><span id=l0>L</span><br><span id=l1>L</span><br><span id=l2>L</span><br></div>",
        &[
            ("m", [8.0, 8.0, 200.0, 20.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("b", [108.0, 8.0, 10.0, 10.0]),
            ("s", [8.0, 18.0, 200.0, 10.0]),
            ("l0", [8.0, 28.0, 10.0, 10.0]),
            ("l1", [108.0, 28.0, 10.0, 10.0]),
            ("l2", [208.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn the_margins_of_the_spanner_count_against_a_definite_height() {
    assert_boxes(
        "<div id=m style='orphans:1;widows:1;columns:2;column-gap:0;width:200px;height:60px'><span id=a>A</span><br><span id=b>B</span><div id=s style='column-span:all;height:10px;margin:5px 0'></div><span id=l0>L</span><br><span id=l1>L</span><br><span id=l2>L</span><br><span id=l3>L</span><br></div>",
        &[
            ("m", [8.0, 8.0, 200.0, 60.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("b", [108.0, 8.0, 10.0, 10.0]),
            ("s", [8.0, 23.0, 200.0, 10.0]),
            ("l0", [8.0, 38.0, 10.0, 10.0]),
            ("l1", [8.0, 48.0, 10.0, 10.0]),
            ("l2", [108.0, 38.0, 10.0, 10.0]),
            ("l3", [108.0, 48.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn the_last_group_overflows_into_more_columns_past_a_maximum_height() {
    assert_boxes(
        "<div id=m style='orphans:1;widows:1;columns:2;column-gap:0;width:200px;max-height:40px'><span id=a>A</span><br><span id=b>B</span><div id=s style='column-span:all;height:10px'></div><span id=l0>L</span><br><span id=l1>L</span><br><span id=l2>L</span><br><span id=l3>L</span><br><span id=l4>L</span><br><span id=l5>L</span><br><span id=l6>L</span><br><span id=l7>L</span><br><span id=l8>L</span><br><span id=l9>L</span><br></div>",
        &[
            ("m", [8.0, 8.0, 200.0, 40.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("b", [108.0, 8.0, 10.0, 10.0]),
            ("s", [8.0, 18.0, 200.0, 10.0]),
            ("l0", [8.0, 28.0, 10.0, 10.0]),
            ("l1", [8.0, 38.0, 10.0, 10.0]),
            ("l2", [108.0, 28.0, 10.0, 10.0]),
            ("l3", [108.0, 38.0, 10.0, 10.0]),
            ("l4", [208.0, 28.0, 10.0, 10.0]),
            ("l5", [208.0, 38.0, 10.0, 10.0]),
            ("l6", [308.0, 28.0, 10.0, 10.0]),
            ("l7", [308.0, 38.0, 10.0, 10.0]),
            ("l8", [408.0, 28.0, 10.0, 10.0]),
            ("l9", [408.0, 38.0, 10.0, 10.0]),
        ],
    );
}
