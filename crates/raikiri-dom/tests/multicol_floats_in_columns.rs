//! A multicol container with a definite height whose children include a
//! float lays the float out in its column: the float shrinks to the column,
//! sits at the column's edge, and the boxes after it flow beside it in the
//! column, as in a block container of the column's width (CSS Multi-column 1,
//! 6.1; CSS 2, 9.5). Content past the last column goes on in overflow columns.
//! Balanced columns are as tall as their content needs, up to the container's
//! height, and a column end cuts a sized box with nothing inside it.
//!
//! The expected rectangles are the ones Chromium gives the same markup in an
//! 800px by 600px viewport with a body margin of 8px, `div` boxes sized by
//! their border box and the Ahem font at 20px. Each is the left, top, width
//! and height of the border box of an element with an `id`; the one of a box
//! in several columns is the union of its pieces. Every container is 100px
//! wide with two 40px columns and a 20px gap.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The left, top, width and height of a box.
type Rect = (f32, f32, f32, f32);

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");

/// The union of the pieces of each element with an `id` on the first page.
fn rects_of(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><html><head><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:8px;font:20px/20px Ahem}}div{{box-sizing:border-box}}</style>\
         </head><body>{body}</body></html>"
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
    let mut rects: Vec<(String, Rect)> = Vec::new();
    for fragment in page.fragments() {
        let Some(id) = dom.attr(fragment.node(), "id") else {
            continue;
        };
        let rect = fragment.rect();
        let (right, bottom) = (rect.x + rect.width, rect.y + rect.height);
        match rects.iter_mut().find(|(seen, _)| seen == id) {
            Some((_, own)) => {
                let (x0, y0) = (own.0.min(rect.x), own.1.min(rect.y));
                let (x1, y1) = ((own.0 + own.2).max(right), (own.1 + own.3).max(bottom));
                *own = (x0, y0, x1 - x0, y1 - y0);
            }
            None => rects.push((id.to_owned(), (rect.x, rect.y, rect.width, rect.height))),
        }
    }
    rects
}

/// Lay out `content` in a 40px tall container of each `column-fill` and
/// compare the box of each `id` with `expected`.
fn check(content: &str, expected: &[(&str, Rect)]) {
    check_fills(content, &["auto", "balance"], expected);
}

/// Lay out `content` in a 40px tall container of each of `fills` and compare
/// the box of each `id` with `expected`.
fn check_fills(content: &str, fills: &[&str], expected: &[(&str, Rect)]) {
    let containers: Vec<String> = fills
        .iter()
        .map(|fill| format!("height:40px;column-fill:{fill}"))
        .collect();
    let containers: Vec<&str> = containers.iter().map(String::as_str).collect();
    check_containers(content, &containers, expected);
}

/// Lay out `content` in a container with each of the extra declarations in
/// `containers` and compare the box of each `id` with `expected`.
fn check_containers(content: &str, containers: &[&str], expected: &[(&str, Rect)]) {
    for container in containers {
        let rects = rects_of(&format!(
            "<div id=m style=\"width:100px;column-count:2;column-gap:20px;{container}\">\
             {content}</div>"
        ));
        for &(id, want) in expected {
            let got = rects
                .iter()
                .find(|(seen, _)| seen == id)
                .map(|(_, rect)| *rect)
                .unwrap_or_else(|| panic!("no box for #{id} in {rects:?}"));
            assert_eq!(got, want, "#{id} with {container}");
        }
    }
}

/// A right float sits at the right edge of the first column, not of the
/// container, and the paragraph after it wraps below it and goes on in the
/// second column.
#[test]
fn a_float_beside_the_paragraph_after_it_goes_to_the_edge_of_its_column() {
    check(
        "<div style=\"float:right;width:20px;height:20px\" id=f></div><p id=p style=\"margin:0\">A B</p>",
        &[
            ("f", (28.0, 8.0, 20.0, 20.0)),
            ("p", (8.0, 8.0, 40.0, 40.0)),
        ],
    );
}

/// Two floats are each shrunk to fit the column: the first wraps its two words,
/// and the second goes on in the second column.
#[test]
fn floats_after_each_other_shrink_to_the_column() {
    check(
        "<p id=f style=\"float:left;margin:0\">A B</p><p id=g style=\"float:left;margin:0\">C</p>",
        &[
            ("f", (8.0, 8.0, 40.0, 40.0)),
            ("g", (68.0, 8.0, 20.0, 20.0)),
        ],
    );
}

/// The paragraph after a float flows beside it and its lines past the column's
/// end go on in the next column.
#[test]
fn a_paragraph_after_the_column_floats_flows_beside_them() {
    check(
        "<p id=p style=\"margin:0\">A</p><div id=f style=\"float:right;width:20px;height:20px\"></div><p id=q style=\"margin:0\">B C</p>",
        &[
            ("p", (8.0, 8.0, 40.0, 20.0)),
            ("f", (28.0, 28.0, 20.0, 20.0)),
            ("q", (68.0, 8.0, 40.0, 40.0)),
        ],
    );
}

/// An `overflow: hidden` box takes the room the float leaves in its column.
#[test]
fn a_box_with_its_own_context_sits_beside_the_float_in_its_column() {
    check(
        "<div id=f style=\"float:left;width:20px;height:20px\"></div><div id=b style=\"overflow:hidden;height:30px\"></div>",
        &[
            ("f", (8.0, 8.0, 20.0, 20.0)),
            ("b", (28.0, 8.0, 20.0, 30.0)),
        ],
    );
}

const CLEARED: &str = "<div id=f style=\"float:left;width:20px;height:20px\"></div><div id=b style=\"clear:both;height:20px\"></div><p id=p style=\"margin:0\">A</p>";

/// A block that clears the float starts below it, and the paragraph after it
/// goes on in the second column.
#[test]
fn a_cleared_block_goes_below_the_float_in_its_column() {
    check_fills(
        CLEARED,
        &["auto"],
        &[
            ("f", (8.0, 8.0, 20.0, 20.0)),
            ("b", (8.0, 28.0, 40.0, 20.0)),
            ("p", (68.0, 8.0, 40.0, 20.0)),
        ],
    );
}

/// Balanced columns are as tall as their content needs, whatever height the
/// container has (CSS Multi-column 1, 8.1): the 60px of content go into 30px
/// columns, which break the cleared block between them. So do 30px columns
/// that fill in turn.
#[test]
fn balanced_columns_with_a_float_break_the_cleared_block_after_it() {
    check_containers(
        CLEARED,
        &[
            "",
            "height:40px;column-fill:balance",
            "height:70px;column-fill:balance",
            "height:200px;column-fill:balance",
            "height:30px;column-fill:auto",
        ],
        &[
            ("f", (8.0, 8.0, 20.0, 20.0)),
            ("b", (8.0, 8.0, 100.0, 30.0)),
            ("p", (68.0, 18.0, 40.0, 20.0)),
        ],
    );
    check_containers(CLEARED, &[""], &[("m", (8.0, 8.0, 100.0, 30.0))]);
    check_containers(
        CLEARED,
        &["height:70px;column-fill:balance"],
        &[("m", (8.0, 8.0, 100.0, 70.0))],
    );
}

/// Lines beside a float balance to the same columns in a taller container
/// as in one whose height is auto.
#[test]
fn balanced_columns_with_a_float_do_not_grow_with_the_container() {
    check_containers(
        "<div id=f style=\"float:right;width:20px;height:20px\"></div><p id=p style=\"margin:0\">A B C</p>",
        &[
            "",
            "height:50px;column-fill:balance",
            "height:70px;column-fill:balance",
            "height:200px;column-fill:balance",
        ],
        &[
            ("f", (28.0, 8.0, 20.0, 20.0)),
            ("p", (8.0, 8.0, 100.0, 40.0)),
        ],
    );
}

/// A sized box with nothing inside it is cut where a balanced column ends,
/// with no float before it too.
#[test]
fn balanced_columns_break_a_sized_box_between_them() {
    check_containers(
        "<div id=x style=\"height:20px\"></div><div id=b style=\"height:20px\"></div><p id=p style=\"margin:0\">A</p>",
        &[
            "",
            "height:40px;column-fill:balance",
            "height:30px;column-fill:auto",
        ],
        &[
            ("x", (8.0, 8.0, 40.0, 20.0)),
            ("b", (8.0, 8.0, 100.0, 30.0)),
            ("p", (68.0, 18.0, 40.0, 20.0)),
        ],
    );
    check_containers(
        "<div id=x style=\"height:20px\"></div><div id=b style=\"height:20px\"></div><p id=p style=\"margin:0\">A</p>",
        &[""],
        &[("m", (8.0, 8.0, 100.0, 30.0))],
    );
}

/// Lines that do not fit the two columns go on in overflow columns past the
/// container's right edge.
#[test]
fn lines_beside_a_float_go_on_in_overflow_columns() {
    check(
        "<div id=f style=\"float:left;width:20px;height:20px\"></div><p id=p style=\"margin:0\">A B C D E F G H I</p>",
        &[
            ("f", (8.0, 8.0, 20.0, 20.0)),
            ("p", (8.0, 8.0, 280.0, 40.0)),
        ],
    );
}

/// A left and a right float share the top of the first column, and the
/// paragraph after them wraps below them and goes on in the second column.
#[test]
fn floats_on_both_sides_share_the_top_of_the_column() {
    check(
        "<div id=f style=\"float:left;width:10px;height:10px\"></div>\
         <div id=g style=\"float:right;width:10px;height:10px\"></div>\
         <p id=p style=\"margin:0\">A B</p>",
        &[
            ("f", (8.0, 8.0, 10.0, 10.0)),
            ("g", (38.0, 8.0, 10.0, 10.0)),
            ("p", (8.0, 8.0, 40.0, 40.0)),
        ],
    );
}
