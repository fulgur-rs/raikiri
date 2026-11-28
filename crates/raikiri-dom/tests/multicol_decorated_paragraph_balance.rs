//! Paragraphs with block-start or block-end padding or border that balanced
//! multi-column containers break between their lines.
//!
//! The expected rectangles are what Chromium reports through
//! `getClientRects()` for each fragment, with a zero body margin. The edge
//! that a break cuts is sliced off the piece: a piece that the paragraph goes
//! on from runs to its column's end, which clips the box, so the rectangles
//! compared here are the parts of the pieces inside the container.
//!
//! https://drafts.csswg.org/css-multicol-1/#cf
//! https://drafts.csswg.org/css-break-3/#break-decoration

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

type Rect = [f32; 4];

/// Markup, and the expected pieces of each element by `id`.
type Case<'a> = (&'a str, &'a [(&'a str, &'a [Rect])]);

/// The fragments of each element with an `id` in `body`, inside the
/// multi-column container `mc`, on the first page of 800px by 600px.
fn pieces(body: &str) -> Vec<(String, Vec<Rect>)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box; margin: 0 }}</style>{body}"
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
    let container = page
        .fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some("mc"))
        .expect("the container")
        .rect();
    let mut found: Vec<(String, Vec<Rect>)> = Vec::new();
    for fragment in page.fragments() {
        let Some(id) = dom.attr(fragment.node(), "id").filter(|id| *id != "mc") else {
            continue;
        };
        let rect = fragment.rect();
        let top = rect.y.max(container.y);
        let bottom = (rect.y + rect.height).min(container.y + container.height);
        let visible = [rect.x, top, rect.width, (bottom - top).max(0.0)];
        match found.iter_mut().find(|(name, _)| name == id) {
            Some((_, rects)) => rects.push(visible),
            None => found.push((id.to_string(), vec![visible])),
        }
    }
    found
}

fn assert_pieces(body: &str, expected: &[(&str, &[Rect])]) {
    let found = pieces(body);
    for (id, rects) in expected {
        let actual = found
            .iter()
            .find(|(name, _)| name == id)
            .map(|(_, rects)| rects.as_slice())
            .unwrap_or_default();
        let same = actual.len() == rects.len()
            && actual.iter().zip(rects.iter()).all(|(actual, expected)| {
                actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| (actual - expected).abs() < 0.01)
            });
        assert!(same, "{body}\n{id}: expected {rects:?}, got {actual:?}");
    }
}

#[test]
fn balanced_columns_break_between_the_lines_of_a_decorated_paragraph() {
    let cases: &[Case] = &[
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='padding-bottom:5px'>X<br>X<br>X<br>X</div></div>",
            &[("p", &[[0.0, 0.0, 100.0, 25.0], [100.0, 0.0, 100.0, 25.0]])],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=q>Y<br>Y</div><div id=p style='padding-bottom:5px'>X<br>X<br>X<br>X</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 20.0]]),
                ("p", &[[0.0, 20.0, 100.0, 20.0], [100.0, 0.0, 100.0, 25.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='padding-top:5px'>X<br>X<br>X<br>X</div></div>",
            &[("p", &[[0.0, 0.0, 100.0, 25.0], [100.0, 0.0, 100.0, 20.0]])],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='padding-top:5px'>X<br>X<br>X<br>X</div><div id=q>Y<br>Y</div></div>",
            &[
                ("p", &[[0.0, 0.0, 100.0, 40.0], [100.0, 0.0, 100.0, 20.0]]),
                ("q", &[[100.0, 20.0, 100.0, 20.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=q>Y<br>Y</div><div id=p style='padding-top:5px'>X<br>X<br>X<br>X</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 20.0]]),
                ("p", &[[0.0, 20.0, 100.0, 25.0], [100.0, 0.0, 100.0, 20.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='border-top:5px solid'>X<br>X<br>X<br>X</div></div>",
            &[("p", &[[0.0, 0.0, 100.0, 25.0], [100.0, 0.0, 100.0, 20.0]])],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='border-top:5px solid'>X<br>X<br>X<br>X</div><div id=q>Y<br>Y</div></div>",
            &[
                ("p", &[[0.0, 0.0, 100.0, 40.0], [100.0, 0.0, 100.0, 20.0]]),
                ("q", &[[100.0, 20.0, 100.0, 20.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=q>Y<br>Y</div><div id=p style='border-top:5px solid'>X<br>X<br>X<br>X</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 20.0]]),
                ("p", &[[0.0, 20.0, 100.0, 25.0], [100.0, 0.0, 100.0, 20.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='padding:5px 0'>X<br>X<br>X<br>X</div></div>",
            &[("p", &[[0.0, 0.0, 100.0, 25.0], [100.0, 0.0, 100.0, 25.0]])],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='padding:5px 0'>X<br>X<br>X<br>X</div><div id=q>Y<br>Y</div></div>",
            &[
                ("p", &[[0.0, 0.0, 100.0, 45.0], [100.0, 0.0, 100.0, 25.0]]),
                ("q", &[[100.0, 25.0, 100.0, 20.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=q>Y<br>Y</div><div id=p style='padding:5px 0'>X<br>X<br>X<br>X</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 20.0]]),
                ("p", &[[0.0, 20.0, 100.0, 25.0], [100.0, 0.0, 100.0, 25.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=q>Y<br>Y</div><div id=p style='padding:5px 0;border:2px solid'>X<br>X<br>X</div><div id=r>Z</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 20.0]]),
                ("p", &[[0.0, 20.0, 100.0, 27.0], [100.0, 0.0, 100.0, 17.0]]),
                ("r", &[[100.0, 17.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=q>Y<br>Y</div><div id=p style='padding:5px 0;box-decoration-break:clone'>X<br>X<br>X</div><div id=r>Z</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 20.0]]),
                ("p", &[[0.0, 20.0, 100.0, 30.0], [100.0, 0.0, 100.0, 20.0]]),
                ("r", &[[100.0, 20.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=q>Y<br>Y</div><div id=p style='padding-bottom:5px;margin-bottom:10px'>X<br>X<br>X<br>X</div><div id=r>Z</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 20.0]]),
                ("p", &[[0.0, 20.0, 100.0, 25.0], [100.0, 0.0, 100.0, 25.0]]),
                ("r", &[[100.0, 35.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px'><div id=q>Y<br>Y</div><div id=p style='border-bottom:3px solid'>X<br>X<br>X</div><div id=r>Z</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 20.0]]),
                ("p", &[[100.0, 0.0, 100.0, 23.0], [200.0, 0.0, 100.0, 13.0]]),
                ("r", &[[200.0, 13.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px'><div id=q>Y<br>Y</div><div id=p style='border-bottom:3px solid'>X<br>X<br>X<br>X</div><div id=r>Z</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 20.0]]),
                ("p", &[[100.0, 0.0, 100.0, 33.0], [200.0, 0.0, 100.0, 23.0]]),
                ("r", &[[200.0, 23.0, 100.0, 10.0]]),
            ],
        ),
    ];
    for (body, expected) in cases {
        assert_pieces(body, expected);
    }
}

#[test]
fn cloned_edges_repeat_in_each_piece_of_a_container_with_a_definite_height() {
    assert_pieces(
        "<div id=mc style='columns:2;column-gap:0;width:200px;height:60px'><div id=p style='padding:5px 0;box-decoration-break:clone'>X<br>X<br>X<br>X</div></div>",
        &[("p", &[[0.0, 0.0, 100.0, 30.0], [100.0, 0.0, 100.0, 30.0]])],
    );
    assert_pieces(
        "<div id=mc style='columns:2;column-gap:0;width:200px;height:60px'><div id=q>Y<br>Y</div><div id=p style='padding:5px 0;box-decoration-break:clone'>X<br>X<br>X</div></div>",
        &[
            ("q", &[[0.0, 0.0, 100.0, 20.0]]),
            ("p", &[[100.0, 0.0, 100.0, 40.0]]),
        ],
    );
}

#[test]
fn balanced_columns_keep_the_orphans_of_the_first_piece_and_yield_its_widows() {
    let cases: &[Case] = &[
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px'><div id=p style='padding-bottom:5px'>X<br>X<br>X<br>X</div></div>",
            &[(
                "p",
                &[
                    [0.0, 0.0, 100.0, 20.0],
                    [100.0, 0.0, 100.0, 20.0],
                    [200.0, 0.0, 100.0, 15.0],
                ],
            )],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='padding-top:7px'>X<br>X<br>X</div></div>",
            &[("p", &[[0.0, 0.0, 100.0, 27.0], [100.0, 0.0, 100.0, 10.0]])],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='padding-bottom:30px'>X<br>X</div></div>",
            &[("p", &[[0.0, 0.0, 100.0, 40.0], [100.0, 0.0, 100.0, 40.0]])],
        ),
        (
            "<div id=mc style='columns:2;column-gap:0;width:200px'><div id=p style='padding-top:30px'>X<br>X</div></div>",
            &[("p", &[[0.0, 0.0, 100.0, 50.0]])],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px'><div id=p style='padding:5px 0;box-decoration-break:clone'>X<br>X<br>X<br>X<br>X<br>X</div></div>",
            &[(
                "p",
                &[
                    [0.0, 0.0, 100.0, 30.0],
                    [100.0, 0.0, 100.0, 30.0],
                    [200.0, 0.0, 100.0, 30.0],
                ],
            )],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px'><div id=p style='border-bottom:30px solid'>X<br>X<br>X<br>X</div></div>",
            &[(
                "p",
                &[
                    [0.0, 0.0, 100.0, 40.0],
                    [100.0, 0.0, 100.0, 40.0],
                    [200.0, 0.0, 100.0, 40.0],
                ],
            )],
        ),
    ];
    for (body, expected) in cases {
        assert_pieces(body, expected);
    }
}
