//! The columns of a multicolumn container with a definite height and the
//! default `column-fill: balance` are as tall as their content needs, up to
//! that height: the content is shared among them, and the container keeps its
//! height (CSS Multi-column 1 sections 8.1 and 8.2). Content that needs more
//! than the container's height fills its columns and goes on in overflow
//! columns, and `column-fill: auto` fills the columns in turn.
//!
//! The expected pieces are the ones Chromium gives the same markup in an 800px
//! by 600px viewport with the body margin of 8px and 10px Ahem text: each is
//! the left, top, width and height of a client rectangle of the element.

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

/// `count` empty blocks of `height` px, with the ids `b0`, `b1`, and so on.
fn blocks(count: usize, height: u32) -> String {
    (0..count)
        .map(|index| format!("<div id=b{index} style=\"height:{height}px\"></div>"))
        .collect()
}

/// `count` lines of text.
fn lines(count: usize) -> String {
    vec!["L"; count].join("<br>")
}

/// `count` lines of text, each in a span with the id `l0`, `l1`, and so on.
fn spans(count: usize) -> String {
    (0..count)
        .map(|index| format!("<span id=l{index}>L</span>"))
        .collect::<Vec<_>>()
        .join("<br>")
}

/// The pieces of each element with an `id` on the first page, ordered by
/// their left and top.
fn pieces_of(body: &str) -> Vec<(String, Vec<Piece>)> {
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
    let mut pieces: Vec<(String, Vec<Piece>)> = Vec::new();
    for fragment in page.fragments() {
        let Some(id) = dom.attr(fragment.node(), "id") else {
            continue;
        };
        let rect = fragment.rect();
        let piece = (rect.x, rect.y, rect.width, rect.height);
        match pieces.iter_mut().find(|(seen, _)| seen == id) {
            Some((_, own)) => own.push(piece),
            None => pieces.push((id.to_owned(), vec![piece])),
        }
    }
    for (_, own) in &mut pieces {
        own.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    }
    pieces
}

/// Lays out `body` and compares the pieces of each element in `expected`.
fn check(label: &str, body: &str, expected: &[(&str, &[Piece])]) {
    let pieces = pieces_of(body);
    for &(id, expected) in expected {
        let (_, got) = pieces
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
        assert_eq!(got.len(), expected.len(), "{label}: #{id} pieces {got:?}");
        for (got, expected) in got.iter().zip(expected) {
            assert!(
                (got.0 - expected.0).abs() <= 0.05
                    && (got.1 - expected.1).abs() <= 0.05
                    && (got.2 - expected.2).abs() <= 0.05
                    && (got.3 - expected.3).abs() <= 0.05,
                "{label}: #{id} piece {got:?}, expected {expected:?}"
            );
        }
    }
}

/// Boxes with no lines are shared among the balanced columns, and cut where a column ends.
#[test]
fn boxes_without_lines_share_the_columns() {
    check(
        "two_blocks",
        &format!("<div style=\"{TWO};height:100px\">{}</div>", blocks(2, 40)),
        &[
            ("b0", &[(8.0, 8.0, 100.0, 40.0)]),
            ("b1", &[(108.0, 8.0, 100.0, 40.0)]),
        ],
    );
    check(
        "blocks_exact_fill",
        &format!("<div style=\"{TWO};height:100px\">{}</div>", blocks(2, 50)),
        &[
            ("b0", &[(8.0, 8.0, 100.0, 50.0)]),
            ("b1", &[(108.0, 8.0, 100.0, 50.0)]),
        ],
    );
    check(
        "blocks_sliced_in_three_columns",
        &format!(
            "<div style=\"columns:3;width:300px;column-gap:0;height:200px\">{}</div>",
            blocks(5, 20)
        ),
        &[
            ("b0", &[(8.0, 8.0, 100.0, 20.0)]),
            (
                "b1",
                &[(8.0, 28.0, 100.0, 13.34), (108.0, 8.0, 100.0, 6.66)],
            ),
            ("b2", &[(108.0, 14.66, 100.0, 20.0)]),
            (
                "b3",
                &[(108.0, 34.66, 100.0, 6.69), (208.0, 8.0, 100.0, 13.31)],
            ),
            ("b4", &[(208.0, 21.31, 100.0, 20.0)]),
        ],
    );
    check(
        "blocks_in_a_padded_box",
        &format!(
            "<div style=\"{TWO};height:100px;padding:5px 3px;border:2px solid\">{}</div>",
            blocks(2, 40)
        ),
        &[
            ("b0", &[(13.0, 15.0, 95.0, 40.0)]),
            ("b1", &[(108.0, 15.0, 95.0, 40.0)]),
        ],
    );
}

/// Content taller than the container fills the columns and goes on in overflow columns, and `column-fill: auto` fills them in turn.
#[test]
fn boxes_without_lines_that_need_more_than_the_height_fill_the_columns() {
    check(
        "blocks_more_than_fits",
        &format!("<div style=\"{TWO};height:100px\">{}</div>", blocks(5, 50)),
        &[
            ("b0", &[(8.0, 8.0, 100.0, 50.0)]),
            ("b1", &[(8.0, 58.0, 100.0, 50.0)]),
            ("b2", &[(108.0, 8.0, 100.0, 50.0)]),
            ("b3", &[(108.0, 58.0, 100.0, 50.0)]),
            ("b4", &[(208.0, 8.0, 100.0, 50.0)]),
        ],
    );
    check(
        "blocks_column_fill_auto",
        &format!(
            "<div style=\"{TWO};height:100px;column-fill:auto\">{}</div>",
            blocks(2, 40)
        ),
        &[
            ("b0", &[(8.0, 8.0, 100.0, 40.0)]),
            ("b1", &[(8.0, 48.0, 100.0, 40.0)]),
        ],
    );
}

/// Paragraphs are shared among the balanced columns, and a paragraph is cut between its lines.
#[test]
fn paragraphs_share_the_columns() {
    check(
        "four_one_line_divs",
        &format!(
            "<div style=\"{TWO};height:200px;font:16px/20px Ahem\"><div id=a>A</div><div id=b>B</div><div id=c>C</div><div id=d>D</div></div>"
        ),
        &[
            ("a", &[(8.0, 8.0, 100.0, 20.0)]),
            ("b", &[(8.0, 28.0, 100.0, 20.0)]),
            ("c", &[(108.0, 8.0, 100.0, 20.0)]),
            ("d", &[(108.0, 28.0, 100.0, 20.0)]),
        ],
    );
    check(
        "paragraph_of_six_lines",
        &format!(
            "<div style=\"{TWO};height:100px\"><p id=p style=\"margin:0\">{}</p></div>",
            lines(6)
        ),
        &[("p", &[(8.0, 8.0, 100.0, 30.0), (108.0, 8.0, 100.0, 30.0)])],
    );
    check(
        "paragraph_of_five_lines",
        &format!(
            "<div style=\"{TWO};height:100px\"><p id=p style=\"margin:0\">{}</p></div>",
            lines(5)
        ),
        &[("p", &[(8.0, 8.0, 100.0, 30.0), (108.0, 8.0, 100.0, 20.0)])],
    );
    check(
        "two_paragraphs",
        &format!(
            "<div style=\"{TWO};height:100px;orphans:1;widows:1\"><p id=p style=\"margin:0\">{}</p><p id=q style=\"margin:0\">{}</p></div>",
            lines(3),
            lines(3)
        ),
        &[
            ("p", &[(8.0, 8.0, 100.0, 30.0)]),
            ("q", &[(108.0, 8.0, 100.0, 30.0)]),
        ],
    );
}

/// A paragraph taller than the container fills the columns and goes on in overflow columns, and `column-fill: auto` fills them in turn.
#[test]
fn paragraphs_that_need_more_than_the_height_fill_the_columns() {
    check(
        "paragraph_more_than_fits",
        &format!(
            "<div style=\"{TWO};height:40px;orphans:1;widows:1\"><p id=p style=\"margin:0\">{}</p></div>",
            lines(12)
        ),
        &[(
            "p",
            &[
                (8.0, 8.0, 100.0, 40.0),
                (108.0, 8.0, 100.0, 40.0),
                (208.0, 8.0, 100.0, 40.0),
            ],
        )],
    );
    check(
        "paragraph_column_fill_auto",
        &format!(
            "<div style=\"{TWO};height:100px;column-fill:auto\"><p id=p style=\"margin:0\">{}</p></div>",
            lines(4)
        ),
        &[("p", &[(8.0, 8.0, 100.0, 40.0)])],
    );
}

/// Lines directly in the container are shared among the balanced columns.
#[test]
fn lines_in_the_container_share_the_columns() {
    check(
        "bare_lines",
        &format!("<div style=\"{TWO};height:100px\">{}</div>", spans(4)),
        &[
            ("l0", &[(8.0, 8.0, 10.0, 10.0)]),
            ("l1", &[(8.0, 18.0, 10.0, 10.0)]),
            ("l2", &[(108.0, 8.0, 10.0, 10.0)]),
            ("l3", &[(108.0, 18.0, 10.0, 10.0)]),
        ],
    );
    check(
        "bare_odd_lines",
        &format!(
            "<div style=\"{TWO};height:100px;orphans:1;widows:1\">{}</div>",
            spans(5)
        ),
        &[
            ("l0", &[(8.0, 8.0, 10.0, 10.0)]),
            ("l1", &[(8.0, 18.0, 10.0, 10.0)]),
            ("l2", &[(8.0, 28.0, 10.0, 10.0)]),
            ("l3", &[(108.0, 8.0, 10.0, 10.0)]),
            ("l4", &[(108.0, 18.0, 10.0, 10.0)]),
        ],
    );
}

/// Lines taller than the container fill the columns and go on in overflow columns, and `column-fill: auto` fills them in turn.
#[test]
fn lines_in_the_container_that_need_more_than_the_height_fill_the_columns() {
    check(
        "bare_lines_more_than_fits",
        &format!(
            "<div style=\"{TWO};height:30px;orphans:1;widows:1\">{}</div>",
            spans(10)
        ),
        &[
            ("l0", &[(8.0, 8.0, 10.0, 10.0)]),
            ("l1", &[(8.0, 18.0, 10.0, 10.0)]),
            ("l2", &[(8.0, 28.0, 10.0, 10.0)]),
            ("l3", &[(108.0, 8.0, 10.0, 10.0)]),
            ("l4", &[(108.0, 18.0, 10.0, 10.0)]),
            ("l5", &[(108.0, 28.0, 10.0, 10.0)]),
            ("l6", &[(208.0, 8.0, 10.0, 10.0)]),
            ("l7", &[(208.0, 18.0, 10.0, 10.0)]),
            ("l8", &[(208.0, 28.0, 10.0, 10.0)]),
            ("l9", &[(308.0, 8.0, 10.0, 10.0)]),
        ],
    );
    check(
        "bare_lines_column_fill_auto",
        &format!(
            "<div style=\"{TWO};height:100px;column-fill:auto\">{}</div>",
            spans(4)
        ),
        &[
            ("l0", &[(8.0, 8.0, 10.0, 10.0)]),
            ("l1", &[(8.0, 18.0, 10.0, 10.0)]),
            ("l2", &[(8.0, 28.0, 10.0, 10.0)]),
            ("l3", &[(8.0, 38.0, 10.0, 10.0)]),
        ],
    );
}
