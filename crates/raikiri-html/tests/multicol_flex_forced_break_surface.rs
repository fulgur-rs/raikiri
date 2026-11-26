//! Forced column breaks at a flex container in a multi-column container, and
//! between its items: a break before or after the container moves it or what
//! follows it to the next column, a break between the items of a single-line
//! column container or between the lines of a row container starts the next
//! column inside it, and a break before the first item or after the last is
//! one before or after the container (CSS Flexbox 1, 12; CSS Fragmentation
//! 3, 3.1). The values are the ones Chromium gives for the same documents.

use std::collections::BTreeMap;

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf");

type Rect = (i32, i32, i32, i32);

/// The visible parts `(x, y, width, height)` of the fragments of each element
/// with an id, in a document whose columns are 100px wide. A piece of the
/// flex container `f` reaches from its top to the end of the box, so each
/// piece shows what the next one does not, and the boxes in `f` show only
/// where the piece in their column does. Elements with ids starting with `o`
/// are outside `f`.
fn visible(body: &str) -> BTreeMap<String, Vec<Rect>> {
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
    let mut rects = BTreeMap::<String, Vec<(f32, f32, f32, f32)>>::new();
    for fragment in page.fragments() {
        if let Some(id) = dom.attr(fragment.node(), "id") {
            let r = fragment.rect();
            rects
                .entry(id.to_owned())
                .or_default()
                .push((r.x, r.y, r.width, r.height));
        }
    }
    let mut pieces = rects.get("f").cloned().unwrap_or_default();
    pieces.sort_by(|a, b| a.0.total_cmp(&b.0));
    let column = |x: f32| (x / 100.0).floor() as i32;
    let bands: BTreeMap<i32, (f32, f32)> = pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| {
            let shown = pieces
                .get(index + 1)
                .map_or(piece.3, |next| piece.3 - next.3);
            (column(piece.0), (piece.1, piece.1 + shown))
        })
        .collect();
    let round = |r: (f32, f32, f32, f32)| {
        (
            r.0.round() as i32,
            r.1.round() as i32,
            r.2.round() as i32,
            r.3.round() as i32,
        )
    };
    rects
        .into_iter()
        .map(|(id, rects)| {
            let shown = rects
                .into_iter()
                .filter_map(|r| {
                    let band = (pieces.len() > 1 && !id.starts_with('o'))
                        .then(|| bands.get(&column(r.0)))
                        .flatten();
                    let Some(&(top, bottom)) = band else {
                        return Some(round(r));
                    };
                    let start = r.1.max(top);
                    let end = (r.1 + r.3).min(bottom);
                    (end - start > 0.01).then(|| round((r.0, start, r.2, end - start)))
                })
                .collect();
            (id, shown)
        })
        .collect()
}

/// A multi-column container of four columns 100px wide, 100px tall, around
/// `content`.
fn columns(fill: &str, content: &str) -> String {
    format!(
        "<div style='columns:4;column-gap:0;width:400px;height:100px;column-fill:{fill}'>{content}</div>"
    )
}

fn expect(body: &str, expected: &[(&str, &[Rect])]) {
    let got = visible(body);
    for (id, rects) in expected {
        assert_eq!(
            got.get(*id).map(Vec::as_slice).unwrap_or_default(),
            *rects,
            "{id} in {body}"
        );
    }
}

#[test]
fn a_break_between_items_of_a_column_container_starts_the_next_column() {
    expect(
        &columns(
            "balance",
            "<div id=f style='display:flex;flex-direction:column'><div id=a style='height:30px'></div>\
             <div id=b style='height:20px;break-before:column'></div><div id=c style='height:60px'></div></div>",
        ),
        &[
            (
                "f",
                &[
                    (0, 0, 100, 30),
                    (100, 0, 100, 30),
                    (200, 0, 100, 30),
                    (300, 0, 100, 20),
                ],
            ),
            ("a", &[(0, 0, 100, 30)]),
            ("b", &[(100, 0, 100, 20)]),
            (
                "c",
                &[(100, 20, 100, 10), (200, 0, 100, 30), (300, 0, 100, 20)],
            ),
        ],
    );
    // A break after an item, and one before an item in reverse order.
    expect(
        &columns(
            "balance",
            "<div id=f style='display:flex;flex-direction:column'><div id=a style='height:30px;break-after:column'></div>\
             <div id=b style='height:20px'></div><div id=c style='height:20px;break-before:column'></div></div>\
             <div id=o style='height:10px'></div>",
        ),
        &[
            (
                "f",
                &[(0, 0, 100, 30), (100, 0, 100, 30), (200, 0, 100, 20)],
            ),
            ("b", &[(100, 0, 100, 20)]),
            ("c", &[(200, 0, 100, 20)]),
            ("o", &[(200, 20, 100, 10)]),
        ],
    );
    expect(
        &columns(
            "auto",
            "<div id=o style='height:40px'></div><div id=f style='display:flex;flex-direction:column-reverse'>\
             <div id=a style='height:30px;break-before:column'></div><div id=b style='height:20px'></div></div>",
        ),
        &[
            ("f", &[(0, 40, 100, 60), (100, 0, 100, 30)]),
            ("a", &[(100, 0, 100, 30)]),
            ("b", &[(0, 40, 100, 20)]),
        ],
    );
}

#[test]
fn breaks_start_as_many_columns_as_they_need() {
    // Each item after a break starts a column, past the last one too.
    expect(
        &columns(
            "balance",
            "<div id=f style='display:flex;flex-direction:column'><div id=a style='height:30px'></div>\
             <div id=b style='height:20px;break-before:column'></div><div id=c style='height:20px;break-before:column'></div>\
             <div id=d style='height:20px;break-before:column'></div><div id=e style='height:20px;break-before:column'></div></div>",
        ),
        &[("d", &[(300, 0, 100, 20)]), ("e", &[(400, 0, 100, 20)])],
    );
}

#[test]
fn a_break_inside_an_item_at_its_start_or_end_is_a_break_at_the_item() {
    expect(
        &columns(
            "balance",
            "<div id=f style='display:flex;flex-direction:column'><div id=a style='height:30px'></div>\
             <div id=b><div id=b1 style='height:20px;break-before:column'></div></div><div id=c style='height:20px'></div></div>",
        ),
        &[
            ("a", &[(0, 0, 100, 20), (100, 0, 100, 10)]),
            ("b1", &[(200, 0, 100, 20)]),
            ("c", &[(300, 0, 100, 20)]),
        ],
    );
    expect(
        &columns(
            "auto",
            "<div id=f style='display:flex;flex-direction:column'><div id=a style='height:30px'></div>\
             <div id=b style='padding-top:5px'><div id=b1 style='height:20px;break-before:column'></div></div></div>",
        ),
        &[
            ("f", &[(0, 0, 100, 100), (100, 0, 100, 25)]),
            ("b1", &[(100, 5, 100, 20)]),
        ],
    );
}

#[test]
fn a_break_at_the_first_or_last_item_is_a_break_at_the_container() {
    expect(
        &columns(
            "balance",
            "<div id=o style='height:40px'></div><div id=f style='display:flex;flex-direction:column'>\
             <div id=a style='height:30px'></div><div id=b style='height:20px;break-after:column'></div></div>\
             <div id=o2 style='height:10px'></div>",
        ),
        &[
            ("f", &[(100, 10, 100, 20), (200, 0, 100, 30)]),
            ("b", &[(200, 10, 100, 20)]),
            ("o2", &[(300, 0, 100, 10)]),
        ],
    );
    // A break on the container itself, before and after.
    expect(
        &columns(
            "balance",
            "<div id=o style='height:40px'></div><div id=f style='display:flex;break-before:column'>\
             <div id=a style='height:30px;width:20px'></div></div><div id=o2 style='height:10px;break-before:column'></div>\
             <div id=o3 style='display:flex;break-after:column;height:5px'></div><div id=o4 style='height:5px'></div>",
        ),
        &[
            ("f", &[(100, 0, 100, 30)]),
            ("o2", &[(200, 0, 100, 10)]),
            ("o3", &[(200, 10, 100, 5)]),
            ("o4", &[(300, 0, 100, 5)]),
        ],
    );
}

#[test]
fn a_break_at_an_item_of_a_row_container_falls_before_or_after_its_line() {
    // One line: the break is one before or after the container.
    for item in ["break-before:column", "break-after:column"] {
        let body = columns(
            "auto",
            &format!(
                "<div id=o style='height:40px'></div><div id=f style='display:flex'><div id=a style='height:30px;width:20px'></div>\
                 <div id=b style='height:20px;width:20px;{item}'></div></div><div id=o2 style='height:10px'></div>"
            ),
        );
        let (f, o2): (&[Rect], &[Rect]) = if item.starts_with("break-before") {
            (&[(100, 0, 100, 30)], &[(100, 30, 100, 10)])
        } else {
            (&[(0, 40, 100, 30)], &[(100, 0, 100, 10)])
        };
        expect(&body, &[("f", f), ("o2", o2)]);
    }
    // Several lines: the line starts the next column.
    for (b, c) in [("break-before:column", ""), ("", "break-before:column")] {
        expect(
            &columns(
                "auto",
                &format!(
                    "<div id=o style='height:40px'></div><div id=f style='display:flex;flex-wrap:wrap;width:50px'>\
                     <div id=a style='height:30px;width:30px'></div><div id=b style='height:20px;width:30px;{b}'></div>\
                     <div id=c style='height:10px;width:20px;{c}'></div></div><div id=o2 style='height:10px'></div>"
                ),
            ),
            &[
                ("f", &[(0, 40, 50, 60), (100, 0, 50, 20)]),
                ("b", &[(100, 0, 30, 20)]),
                ("c", &[(130, 0, 20, 10)]),
                ("o2", &[(100, 20, 100, 10)]),
            ],
        );
    }
}
