//! Where the lines of paragraphs that a multicol container holds inside
//! wrappers, such as the items of a list, stand. The positions are the ones
//! Chromium prints for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::BTreeMap;

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;
/// The distance between the lines, which have a `line-height` of 20px.
const PITCH: i32 = 20;

/// Lines `from` through `to`, each labeled `prefix` and its two-digit number.
fn lines_of(prefix: &str, from: u32, to: u32) -> String {
    let lines: Vec<_> = (from..=to).map(|n| format!("{prefix}{n:02}")).collect();
    lines.join("<br>")
}

/// A list item of lines `from` through `to`.
fn li(prefix: &str, from: u32, to: u32) -> String {
    format!("<li>{}</li>", lines_of(prefix, from, to))
}

/// A paragraph of lines `from` through `to`.
fn p(prefix: &str, from: u32, to: u32) -> String {
    format!("<p>{}</p>", lines_of(prefix, from, to))
}

/// The lines of every column of every page, as `p<page> x<left>: <runs>`. A
/// run is the first and last label of lines one after the other, and the top
/// of the first, as in `A01-A04@0`. List markers are left out.
fn columns(body: &str, css: &str) -> Vec<String> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}}\
         body{{font:20px/20px Ahem;margin:0}}.mc{{columns:2;column-gap:0}}p{{margin:0}}\
         ul{{margin:0;padding:0;list-style:none}}{css}</style><body>{body}</body>"
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
    let mut cells: BTreeMap<(u32, i32), Vec<(i32, String)>> = BTreeMap::new();
    for index in 0..8 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            let text = run.text.trim_end();
            if text == "•" {
                continue;
            }
            cells
                .entry((index, run.origin.0.round() as i32))
                .or_default()
                .push(((run.origin.1 - ASCENT).round() as i32, text.to_owned()));
        }
    }
    cells
        .into_iter()
        .map(|((page, left), mut lines)| {
            lines.sort();
            let mut runs: Vec<(String, String, i32, i32)> = Vec::new();
            for (top, text) in lines {
                match runs.last_mut() {
                    Some((_, last, _, bottom))
                        if *bottom + PITCH == top && last[..1] == text[..1] =>
                    {
                        *last = text;
                        *bottom = top;
                    }
                    _ => runs.push((text.clone(), text, top, top)),
                }
            }
            let runs: Vec<_> = runs
                .into_iter()
                .map(|(first, last, top, _)| {
                    if first == last {
                        format!("{first}@{top}")
                    } else {
                        format!("{first}-{last}@{top}")
                    }
                })
                .collect();
            format!("p{page} x{left}: {}", runs.join(" "))
        })
        .collect()
}

#[test]
fn the_items_of_a_list_break_between_the_columns() {
    let items = format!("{}{}", li("A", 1, 4), li("B", 1, 4));
    let two = ["p0 x0: A01-A04@0", "p0 x100: B01-B04@0"];
    assert_eq!(
        columns(&format!("<div class=mc><ul>{items}</ul></div>"), ""),
        two
    );
    // A marker sits outside the item, on its first line.
    assert_eq!(
        columns(
            &format!("<div class=mc><ul>{items}</ul></div>"),
            "ul{list-style:disc}"
        ),
        two
    );
    // A marker of an item that moves to the next column moves with it.
    assert_eq!(
        columns(
            &format!("<div class=mc><ul style='padding-left:40px'>{items}</ul></div>"),
            "ul{list-style:'X'}"
        ),
        [
            "p0 x20: X@0",
            "p0 x40: A01-A04@0",
            "p0 x120: X@0",
            "p0 x140: B01-B04@0"
        ]
    );
    let long = format!("{}{}", li("A", 1, 12), li("B", 1, 12));
    assert_eq!(
        columns(
            &format!("<div class=mc><ul style='padding-left:40px'>{long}</ul></div>"),
            "ul{list-style:'X'}"
        ),
        [
            "p0 x20: X@0",
            "p0 x40: A01-A10@0",
            "p0 x120: X@40",
            "p0 x140: A11-A12@0 B01-B08@40",
            "p1 x40: B09-B10@0",
            "p1 x140: B11-B12@0"
        ]
    );
    // The padding at the side of the list moves its items in each column.
    assert_eq!(
        columns(
            &format!("<div class=mc><ul style='padding-left:20px'>{items}</ul></div>"),
            ""
        ),
        ["p0 x20: A01-A04@0", "p0 x120: B01-B04@0"]
    );
    // The margins of the list adjoin those of its first and last items; the
    // one above stays in the first column, the one below is divided with the
    // content.
    assert_eq!(
        columns(
            &format!("<div class=mc><ul style='margin:20px 0'>{items}</ul></div>"),
            ""
        ),
        ["p0 x0: A01-A04@20", "p0 x100: B01-B04@0"]
    );
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><ul style='margin:20px 0;padding-left:40px'>{items}</ul></div>"
            ),
            ""
        ),
        ["p0 x40: A01-A04@20", "p0 x140: B01-B04@0"]
    );
}

#[test]
fn a_single_list_item_breaks_between_its_lines() {
    assert_eq!(
        columns(
            &format!("<div class=mc><ul>{}</ul></div>", li("A", 1, 8)),
            ""
        ),
        ["p0 x0: A01-A04@0", "p0 x100: A05-A08@0"]
    );
}

#[test]
fn paragraphs_in_wrappers_balance_like_the_containers_own() {
    let two = ["p0 x0: A01-A04@0", "p0 x100: B01-B04@0"];
    let pair = format!("{}{}", p("A", 1, 4), p("B", 1, 4));
    assert_eq!(
        columns(&format!("<div class=mc><div>{pair}</div></div>"), ""),
        two
    );
    assert_eq!(
        columns(
            &format!("<div class=mc><div><div>{pair}</div></div></div>"),
            ""
        ),
        two
    );
    // A paragraph broken between the columns.
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><div>{}{}{}</div></div>",
                p("A", 1, 3),
                p("B", 1, 3),
                p("C", 1, 3)
            ),
            ""
        ),
        ["p0 x0: A01-A03@0 B01-B02@60", "p0 x100: B03@0 C01-C03@20"]
    );
    // The margins between the paragraphs of a wrapper collapse, and a break
    // between them discards them.
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><div><p style='margin:20px 0'>{}</p><p style='margin:20px 0'>{}</p></div></div>",
                lines_of("A", 1, 3),
                lines_of("B", 1, 3)
            ),
            ""
        ),
        ["p0 x0: A01-A03@20", "p0 x100: B01-B03@0"]
    );
    // The margins at the sides of a wrapper narrow the boxes in each column.
    assert_eq!(
        columns(
            &format!("<div class=mc><div style='margin:0 10px'>{pair}</div></div>"),
            ""
        ),
        ["p0 x10: A01-A04@0", "p0 x110: B01-B04@0"]
    );
    // In three columns.
    assert_eq!(
        columns(
            &format!(
                "<div class=mc style='columns:3'><div>{}{}{}</div></div>",
                p("A", 1, 3),
                p("B", 1, 3),
                p("C", 1, 3)
            ),
            ""
        ),
        [
            "p0 x0: A01-A03@0",
            "p0 x67: B01-B03@0",
            "p0 x133: C01-C03@0"
        ]
    );
    // Right to left, the first column is on the right.
    assert_eq!(
        columns(
            &format!("<div class=mc dir=rtl><div>{pair}</div></div>"),
            ""
        ),
        ["p0 x40: B01-B04@0", "p0 x140: A01-A04@0"]
    );
}

#[test]
fn wrapped_paragraphs_flow_with_the_boxes_around_them() {
    // A paragraph of the container after a list.
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><ul>{}{}</ul>{}</div>",
                li("A", 1, 4),
                li("B", 1, 4),
                p("C", 1, 2)
            ),
            ""
        ),
        [
            "p0 x0: A01-A04@0 B01-B02@80",
            "p0 x100: B03-B04@0 C01-C02@40"
        ]
    );
    // A paragraph of the container after a wrapper.
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><div>{}{}</div>{}</div>",
                p("A", 1, 4),
                p("B", 1, 4),
                p("C", 1, 4)
            ),
            ""
        ),
        [
            "p0 x0: A01-A04@0 B01-B02@80",
            "p0 x100: B03-B04@0 C01-C04@40"
        ]
    );
}

#[test]
fn nested_lists_break_between_their_items() {
    let inner = format!(
        "<ul style='padding-left:10px'>{}{}</ul>",
        li("B", 1, 3),
        li("C", 1, 3)
    );
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><ul style='padding-left:10px'>{}<li>{inner}</li></ul></div>",
                li("A", 1, 3)
            ),
            ""
        ),
        [
            "p0 x10: A01-A03@0",
            "p0 x20: B01-B02@60",
            "p0 x120: B03@0 C01-C03@20"
        ]
    );
    // An item that holds paragraphs.
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><ul><li>{}{}</li>{}</ul></div>",
                p("A", 1, 3),
                p("B", 1, 3),
                li("C", 1, 3)
            ),
            ""
        ),
        ["p0 x0: A01-A03@0 B01-B02@60", "p0 x100: B03@0 C01-C03@20"]
    );
}

#[test]
fn wrapped_paragraphs_fill_the_columns_of_each_page_before_the_next() {
    let list = format!(
        "<div class=mc><ul>{}{}</ul></div>",
        li("A", 1, 12),
        li("B", 1, 12)
    );
    let pages = [
        "p0 x0: A01-A10@0",
        "p0 x100: A11-A12@0 B01-B08@40",
        "p1 x0: B09-B10@0",
        "p1 x100: B11-B12@0",
    ];
    assert_eq!(columns(&list, ""), pages);
    let wrapped = format!(
        "<div class=mc><div>{}{}</div></div>",
        p("A", 1, 12),
        p("B", 1, 12)
    );
    assert_eq!(columns(&wrapped, ""), pages);
    // One paragraph to each wrapper.
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><div>{}</div><div>{}</div></div>",
                p("L", 1, 25),
                p("L", 26, 50)
            ),
            ""
        ),
        [
            "p0 x0: L01-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L30@0",
            "p1 x100: L31-L40@0",
            "p2 x0: L41-L45@0",
            "p2 x100: L46-L50@0",
        ]
    );
    // One paragraph below two wrappers, and a paragraph after the container.
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><div><div>{}</div></div></div><p>X</p>",
                p("L", 1, 30)
            ),
            ""
        ),
        [
            "p0 x0: L01-L10@0",
            "p0 x100: L11-L20@0",
            "p1 x0: L21-L25@0 X@100",
            "p1 x100: L26-L30@0",
        ]
    );
    // The default margins and padding of a list.
    assert_eq!(
        columns(
            &format!(
                "<div class=mc><ul style='margin:20px 0;padding-left:40px'>{}{}</ul></div>",
                li("A", 1, 12),
                li("B", 1, 12)
            ),
            ""
        ),
        [
            "p0 x40: A01-A09@20",
            "p0 x140: A10-A12@0 B01-B07@60",
            "p1 x40: B08-B10@0",
            "p1 x140: B11-B12@0",
        ]
    );
}

#[test]
fn wrapped_paragraphs_break_between_the_columns_of_a_definite_height() {
    let wrapped = format!("<div>{}{}</div>", p("A", 1, 4), p("B", 1, 4));
    let two = ["p0 x0: A01-A04@0", "p0 x100: B01-B04@0"];
    assert_eq!(
        columns(
            &format!("<div class=mc style='height:100px'>{wrapped}</div>"),
            ""
        ),
        two
    );
    // Filled in turn, the second paragraph would leave one line behind.
    assert_eq!(
        columns(
            &format!("<div class=mc style='height:100px;column-fill:auto'>{wrapped}</div>"),
            ""
        ),
        two
    );
}

#[test]
fn white_space_and_hidden_items_take_no_room_in_the_flow() {
    let items = format!(
        "\n{}\n<li style='display:none'>C01</li>\n{}\n",
        li("A", 1, 4),
        li("B", 1, 4)
    );
    assert_eq!(
        columns(&format!("<div class=mc>\n<ul>{items}</ul>\n</div>"), ""),
        ["p0 x0: A01-A04@0", "p0 x100: B01-B04@0"]
    );
}

#[test]
fn wrappers_nested_past_the_depth_cap_stay_one_block() {
    let pair = format!("{}{}", p("A", 1, 4), p("B", 1, 4));
    let body = format!(
        "<div class=mc>{}{pair}{}</div>",
        "<div>".repeat(40),
        "</div>".repeat(40)
    );
    assert_eq!(columns(&body, ""), ["p0 x0: A01-A04@0 B01-B04@80"]);
}

#[test]
fn nested_lists_fill_the_columns_of_each_page_before_the_next() {
    let items = format!("{}{}", li("A", 1, 12), li("B", 1, 12));
    assert_eq!(
        columns(
            &format!("<div class=mc><ul><li><ul>{items}</ul></li></ul></div>"),
            ""
        ),
        [
            "p0 x0: A01-A10@0",
            "p0 x100: A11-A12@0 B01-B08@40",
            "p1 x0: B09-B10@0",
            "p1 x100: B11-B12@0"
        ]
    );
}
