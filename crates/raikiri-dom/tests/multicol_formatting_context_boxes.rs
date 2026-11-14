//! A box that makes a formatting context of its own (`flow-root`, flex,
//! grid or table) among the children of a multicol container with a height:
//! it is laid out at the width of a column and placed in the columns with the
//! boxes around it, as Chromium places it, instead of at the width of the
//! whole container below the first column.
//!
//! The expected rectangles are the ones Chromium computes for the same markup
//! in an 800px by 600px viewport.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

type Rect = (f32, f32, f32, f32);

fn fragments(body: &str, id: &str) -> Vec<Rect> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
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
    let mut found = Vec::new();
    for page in result.pages() {
        let dom = page.dom();
        for fragment in page.fragments() {
            if dom.attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                found.push((rect.x, rect.y, rect.width, rect.height));
            }
        }
    }
    found
}

/// Asserts that `id` has the fragments `expected` in the layout of `body`.
fn check(body: &str, id: &str, expected: &[Rect]) {
    let got = fragments(body, id);
    let same = got.len() == expected.len()
        && got.iter().zip(expected).all(|(got, expected)| {
            (got.0 - expected.0).abs() <= 0.02
                && (got.1 - expected.1).abs() <= 0.02
                && (got.2 - expected.2).abs() <= 0.02
                && (got.3 - expected.3).abs() <= 0.02
        });
    assert!(
        same,
        "#{id} in {body}: placed {got:?}, expected {expected:?}"
    );
}

/// Two 100px columns, 30px high, with `fill`, holding `lead` and then `rest`.
fn columns(fill: &str, lead: &str, rest: &str) -> String {
    format!(
        "<div style='width:200px;columns:2;column-gap:0;height:30px;column-fill:{fill}'>{lead}{rest}</div>"
    )
}

/// What comes before the box: an empty box 20px high, two paragraphs, or a
/// paragraph of two lines.
const LEADS: [&str; 3] = [
    "<div style='height:20px'></div>",
    "<div>yy</div><div>yy</div>",
    "<p style='margin:0'>yy<br>yy</p>",
];

#[test]
fn a_one_line_box_balances_with_the_boxes_before_it() {
    // The columns balance at 15px. An empty box above is cut there and the
    // box starts 5px down the second column; after lines, it starts the
    // second column.
    for display in ["flow-root", "flex", "grid", "table"] {
        let width = if display == "table" { 20.0 } else { 100.0 };
        for (lead, y) in LEADS.iter().zip([5.0, 0.0, 0.0]) {
            let body = columns(
                "balance",
                lead,
                &format!("<div id=i style='display:{display}'>xx</div>"),
            );
            check(&body, "i", &[(100.0, y, width, 10.0)]);
        }
    }
}

#[test]
fn a_one_line_box_fills_the_first_column_at_its_width() {
    for display in ["flow-root", "flex", "grid", "table"] {
        let width = if display == "table" { 20.0 } else { 100.0 };
        for lead in LEADS {
            let body = columns(
                "auto",
                lead,
                &format!("<div id=i style='display:{display}'>xx</div>"),
            );
            check(&body, "i", &[(0.0, 20.0, width, 10.0)]);
        }
    }
}

#[test]
fn the_boxes_inside_move_with_the_box() {
    let lead = LEADS[0];
    let flex = "<div id=i style='display:flex'><span id=c>xx</span><span id=d>x</span></div>";
    let body = columns("balance", lead, flex);
    check(&body, "c", &[(100.0, 5.0, 20.0, 10.0)]);
    check(&body, "d", &[(120.0, 5.0, 10.0, 10.0)]);
    let body = columns("auto", lead, flex);
    check(&body, "c", &[(0.0, 20.0, 20.0, 10.0)]);
    check(&body, "d", &[(20.0, 20.0, 10.0, 10.0)]);
    let grid = "<div id=i style='display:grid'><span id=c>xx</span></div>";
    check(
        &columns("balance", lead, grid),
        "c",
        &[(100.0, 5.0, 100.0, 10.0)],
    );
    let flow_root = "<div id=i style='display:flow-root'><div id=c>xx</div></div>";
    check(
        &columns("balance", lead, flow_root),
        "c",
        &[(100.0, 5.0, 100.0, 10.0)],
    );
    // A table 12px high does not fit below the empty box in the first column
    // and starts the second one.
    let table = "<div id=i style='display:table'><div id=c style='display:table-cell;padding:1px'>xx</div></div>";
    check(
        &columns("balance", lead, table),
        "c",
        &[(100.0, 4.0, 22.0, 12.0)],
    );
    check(
        &columns("auto", lead, table),
        "c",
        &[(100.0, 0.0, 22.0, 12.0)],
    );
}

#[test]
fn a_flow_root_of_lines_breaks_between_its_lines() {
    // Two lines keep together under the default orphans and widows, so the
    // box starts the second column.
    let two = "<div id=i style='display:flow-root;white-space:pre-line'>xx\nxx</div>";
    let lead = LEADS[0];
    check(
        &columns("balance", lead, two),
        "i",
        &[(100.0, 0.0, 100.0, 20.0)],
    );
    check(
        &columns("auto", lead, two),
        "i",
        &[(100.0, 0.0, 100.0, 20.0)],
    );
    // Four lines with a margin above fill the second column and go on in the
    // overflow column past it.
    let four = "<div id=i style='display:flow-root;white-space:pre-line;margin-top:3px'>xx\nxx\nxx\nxx</div>";
    check(
        &columns("auto", lead, four),
        "i",
        &[(100.0, 0.0, 100.0, 30.0), (200.0, 0.0, 100.0, 20.0)],
    );
}

#[test]
fn margins_do_not_collapse_through_an_empty_flow_root() {
    // The empty box starts a formatting context: its margins stay apart, so
    // the second paragraph does not fit below it and starts the second
    // column.
    let body = columns(
        "balance",
        "<div>yy</div><div id=i style='display:flow-root;margin:10px 0'></div>",
        "<div id=j>yy</div>",
    );
    check(&body, "i", &[(0.0, 20.0, 100.0, 0.0)]);
    check(&body, "j", &[(100.0, 0.0, 100.0, 10.0)]);
}

#[test]
fn a_box_with_room_to_break_inside_is_laid_out_at_the_column_width() {
    // Chromium breaks these boxes between their rows or slices the empty
    // boxes inside them; they still take the width of a column, not of the
    // whole container.
    for (fill, rest) in [
        (
            "balance",
            "<div id=i style='display:flex;flex-wrap:wrap'><span style='width:100px'>a</span><span style='width:100px'>b</span></div>",
        ),
        (
            "auto",
            "<div id=i style='display:flow-root;height:100px'></div>",
        ),
        (
            "auto",
            "<div id=i style='display:flex'><div style='height:25px'></div></div>",
        ),
    ] {
        let body = columns(fill, LEADS[1], rest);
        let got = fragments(&body, "i");
        assert!(
            !got.is_empty() && got.iter().all(|rect| (rect.2 - 100.0).abs() <= 0.02),
            "{body}: {got:?}"
        );
    }
}
