//! A wrapper inside a multicolumn container joins the flow of the columns
//! with what it holds when it has padding above or below its boxes, or holds
//! a sized box with no lines beside its paragraph: the columns balance or fill
//! all of it, cutting the paragraph between its lines and the padding where a
//! column ends (CSS Multi-column 1, section 8; CSS Fragmentation 3, 4.4).
//!
//! The expected pieces are the ones Chromium gives the same markup in an 800px
//! by 600px viewport with the body margin of 8px and 10px Ahem text: each is
//! the left, top, width and height of a client rectangle of the element. For
//! a wrapper only its first rectangle is checked.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// Two columns of 100px with no gap.
const TWO: &str = "columns:2;width:200px;column-gap:0";

/// One piece of a box: its left, top, width and height.
type Piece = (f32, f32, f32, f32);

/// `count` lines of text, each in a span with the id `{prefix}0`,
/// `{prefix}1`, and so on.
fn spans(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|index| format!("<span id={prefix}{index}>x</span>"))
        .collect::<Vec<_>>()
        .join("<br>")
}

/// The pieces of each element with an `id` on the only page.
fn pieces_of(body: &str) -> Vec<(String, Piece)> {
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
    assert_eq!(result.page_count(), 1);
    let page = result.page(0).expect("page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Lays out `body` and checks that each element in `expected` has a piece at
/// each of the rectangles given for it.
fn check(label: &str, body: &str, expected: &[(&str, &[Piece])]) {
    let pieces = pieces_of(body);
    for &(id, rects) in expected {
        for rect in rects {
            assert!(
                pieces.iter().any(|(name, got)| name == id
                    && (got.0 - rect.0).abs() <= 0.05
                    && (got.1 - rect.1).abs() <= 0.05
                    && (got.2 - rect.2).abs() <= 0.05
                    && (got.3 - rect.3).abs() <= 0.05),
                "{label}: no piece of #{id} at {rect:?}; pieces {pieces:?}"
            );
        }
    }
}

/// The pieces of the lines `{prefix}0` and on, each 10px square, at the left
/// and top given for each.
fn lines(prefix: &str, at: &[(f32, f32)]) -> Vec<(String, Vec<Piece>)> {
    at.iter()
        .enumerate()
        .map(|(index, &(x, y))| (format!("{prefix}{index}"), vec![(x, y, 10.0, 10.0)]))
        .collect()
}

/// `check` with the lines of `line_rows` added to `expected`.
fn check_with_lines(
    label: &str,
    body: &str,
    expected: &[(&str, &[Piece])],
    line_rows: &[(String, Vec<Piece>)],
) {
    let mut all: Vec<(&str, &[Piece])> = expected.to_vec();
    all.extend(
        line_rows
            .iter()
            .map(|(id, rects)| (id.as_str(), rects.as_slice())),
    );
    check(label, body, &all);
}

#[test]
fn padding_above_and_below_the_boxes_of_a_wrapper_is_cut_between_the_columns() {
    let two_columns = [
        (8.0, 13.0),
        (8.0, 23.0),
        (8.0, 33.0),
        (108.0, 8.0),
        (108.0, 18.0),
        (108.0, 28.0),
    ];
    for (label, height, container) in [
        ("definite", ";height:100px", (8.0, 8.0, 200.0, 100.0)),
        ("auto", "", (8.0, 8.0, 200.0, 35.0)),
    ] {
        check_with_lines(
            label,
            &format!(
                "<div id=c style=\"{TWO}{height}\"><div id=w style=\"padding:5px 0\">\
                 <div id=p>{}</div></div></div>",
                spans("l", 6)
            ),
            &[
                ("c", &[container]),
                ("w", &[(8.0, 8.0, 100.0, 35.0)]),
                ("p", &[(8.0, 13.0, 100.0, 30.0), (108.0, 8.0, 100.0, 30.0)]),
            ],
            &lines("l", &two_columns),
        );
    }
}

#[test]
fn padding_only_above_the_boxes_keeps_two_paragraphs_in_the_flow() {
    check_with_lines(
        "padding_top",
        &format!(
            "<div id=c style=\"{TWO}\"><div id=w style=\"padding:10px 0 0\">\
             <div id=p>{}</div><div id=q>{}</div></div></div>",
            spans("l", 5),
            spans("m", 3)
        ),
        &[
            ("c", &[(8.0, 8.0, 200.0, 50.0)]),
            ("w", &[(8.0, 8.0, 100.0, 50.0)]),
            ("p", &[(8.0, 18.0, 100.0, 40.0), (108.0, 8.0, 100.0, 20.0)]),
            ("q", &[(108.0, 28.0, 100.0, 30.0)]),
        ],
        &[
            lines(
                "l",
                &[
                    (8.0, 18.0),
                    (8.0, 28.0),
                    (8.0, 38.0),
                    (108.0, 8.0),
                    (108.0, 18.0),
                ],
            ),
            lines("m", &[(108.0, 28.0), (108.0, 38.0), (108.0, 48.0)]),
        ]
        .concat(),
    );
}

#[test]
fn the_padding_of_a_wrapper_keeps_its_margins_apart_from_those_of_its_boxes() {
    check_with_lines(
        "margins",
        &format!(
            "<div id=c style=\"{TWO}\"><div id=w style=\"margin:10px 0;padding:5px 0\">\
             <div id=p style=\"margin:5px 0\">{}</div></div></div>",
            spans("l", 4)
        ),
        &[
            ("c", &[(8.0, 8.0, 200.0, 40.0)]),
            ("w", &[(8.0, 18.0, 100.0, 30.0)]),
            ("p", &[(8.0, 28.0, 100.0, 20.0), (108.0, 8.0, 100.0, 20.0)]),
        ],
        &lines(
            "l",
            &[(8.0, 28.0), (8.0, 38.0), (108.0, 8.0), (108.0, 18.0)],
        ),
    );
}

#[test]
fn a_padded_wrapper_goes_on_in_overflow_columns() {
    check_with_lines(
        "overflow",
        &format!(
            "<div id=c style=\"{TWO};height:30px\"><div id=w style=\"padding:5px 0\">\
             <div id=p>{}</div></div></div>",
            spans("l", 10)
        ),
        &[
            ("c", &[(8.0, 8.0, 200.0, 30.0)]),
            ("w", &[(8.0, 8.0, 100.0, 30.0)]),
            (
                "p",
                &[
                    (8.0, 13.0, 100.0, 25.0),
                    (108.0, 8.0, 100.0, 30.0),
                    (208.0, 8.0, 100.0, 30.0),
                    (308.0, 8.0, 100.0, 20.0),
                ],
            ),
        ],
        &lines(
            "l",
            &[
                (8.0, 13.0),
                (8.0, 23.0),
                (108.0, 8.0),
                (108.0, 18.0),
                (108.0, 28.0),
                (208.0, 8.0),
                (208.0, 18.0),
                (208.0, 28.0),
                (308.0, 8.0),
                (308.0, 18.0),
            ],
        ),
    );
}

#[test]
fn nested_padded_wrappers_join_the_flow() {
    check_with_lines(
        "nested",
        &format!(
            "<div id=c style=\"{TWO}\"><div id=w style=\"padding:5px 0\">\
             <div id=v style=\"padding:5px 0\"><div id=p>{}</div></div></div></div>",
            spans("l", 6)
        ),
        &[
            ("c", &[(8.0, 8.0, 200.0, 40.0)]),
            ("w", &[(8.0, 8.0, 100.0, 40.0)]),
            ("v", &[(8.0, 13.0, 100.0, 35.0)]),
            ("p", &[(8.0, 18.0, 100.0, 30.0), (108.0, 8.0, 100.0, 30.0)]),
        ],
        &lines(
            "l",
            &[
                (8.0, 18.0),
                (8.0, 28.0),
                (8.0, 38.0),
                (108.0, 8.0),
                (108.0, 18.0),
                (108.0, 28.0),
            ],
        ),
    );
}

#[test]
fn a_sized_box_beside_the_paragraph_of_a_wrapper_joins_the_flow() {
    for (label, height, container) in [
        ("definite", ";height:100px", (8.0, 8.0, 200.0, 100.0)),
        ("auto", "", (8.0, 8.0, 200.0, 40.0)),
    ] {
        check_with_lines(
            label,
            &format!(
                "<div id=c style=\"{TWO}{height}\"><div id=w><div id=a style=\"height:40px\">\
                 </div><div id=p>{}</div></div></div>",
                spans("l", 4)
            ),
            &[
                ("c", &[container]),
                ("w", &[(8.0, 8.0, 100.0, 40.0)]),
                ("a", &[(8.0, 8.0, 100.0, 40.0)]),
                ("p", &[(108.0, 8.0, 100.0, 40.0)]),
            ],
            &lines(
                "l",
                &[(108.0, 8.0), (108.0, 18.0), (108.0, 28.0), (108.0, 38.0)],
            ),
        );
    }
}
