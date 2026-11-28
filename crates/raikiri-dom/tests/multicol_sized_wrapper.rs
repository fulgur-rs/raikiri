//! Wrappers with a height of their own (`height`, `min-height`,
//! `max-height`) in multi-column containers: the boxes inside a wrapper whose
//! height is less than its content overflow into the following columns, and a
//! wrapper whose height is more than its content leaves room below them that
//! the columns break like any other block.
//!
//! The expected rectangles are what Chromium reports through
//! `getClientRects()` for each fragment, with a zero body margin. Only the
//! parts of the pieces inside the container are compared, and pieces with no
//! height there are left out.
//!
//! https://drafts.csswg.org/css-multicol-1/#cf
//! https://drafts.csswg.org/css-break-3/#breaking-rules

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

/// Chromium keeps lengths in units of 1/64px, so a column height shared out
/// in thirds differs from the exact share by up to about that much.
const LAYOUT_UNIT: f32 = 1.0 / 32.0;

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
    let mut fragments: Vec<_> = page.fragments().collect();
    fragments.sort_by_key(|fragment| fragment.fragment_index());
    let mut found: Vec<(String, Vec<Rect>)> = Vec::new();
    for fragment in fragments {
        let Some(id) = dom.attr(fragment.node(), "id").filter(|id| *id != "mc") else {
            continue;
        };
        let rect = fragment.rect();
        let top = rect.y.max(container.y);
        let bottom = (rect.y + rect.height).min(container.y + container.height);
        if bottom <= top {
            continue;
        }
        let visible = [rect.x, top, rect.width, bottom - top];
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
                    .all(|(actual, expected)| (actual - expected).abs() < LAYOUT_UNIT)
            });
        assert!(same, "{body}\n{id}: expected {rects:?}, got {actual:?}");
    }
}

#[test]
fn the_boxes_of_a_wrapper_shorter_than_its_content_overflow_into_later_columns() {
    let cases: &[Case] = &[
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=w style='background:#fcc;height:30px'><div id=a style='height:60px'></div></div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 20.0], [100.0, 0.0, 100.0, 10.0]]),
                (
                    "a",
                    &[
                        [0.0, 0.0, 100.0, 20.0],
                        [100.0, 0.0, 100.0, 20.0],
                        [200.0, 0.0, 100.0, 20.0],
                    ],
                ),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=w style='background:#fcc;height:30px'><div id=a>X<br>X<br>X<br>X<br>X<br>X</div></div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 20.0], [100.0, 0.0, 100.0, 10.0]]),
                (
                    "a",
                    &[
                        [0.0, 0.0, 100.0, 20.0],
                        [100.0, 0.0, 100.0, 20.0],
                        [200.0, 0.0, 100.0, 20.0],
                    ],
                ),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;'><div id=w style='background:#fcc;height:30px'><div id=a style='height:20px'></div><div id=b style='height:40px'></div></div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 20.0], [100.0, 0.0, 100.0, 10.0]]),
                ("a", &[[0.0, 0.0, 100.0, 20.0]]),
                ("b", &[[100.0, 0.0, 100.0, 20.0], [200.0, 0.0, 100.0, 20.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;'><div id=w style='background:#fcc;height:30px'><div id=a style='height:50px;margin-bottom:10px'></div></div></div>",
            &[
                (
                    "w",
                    &[[0.0, 0.0, 100.0, 16.671875], [100.0, 0.0, 100.0, 13.328125]],
                ),
                (
                    "a",
                    &[
                        [0.0, 0.0, 100.0, 16.671875],
                        [100.0, 0.0, 100.0, 16.671875],
                        [200.0, 0.0, 100.0, 16.65625],
                    ],
                ),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;'><div id=w style='background:#fcc;min-height:0;height:30px'><div id=a style='height:60px'></div></div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 20.0], [100.0, 0.0, 100.0, 10.0]]),
                (
                    "a",
                    &[
                        [0.0, 0.0, 100.0, 20.0],
                        [100.0, 0.0, 100.0, 20.0],
                        [200.0, 0.0, 100.0, 20.0],
                    ],
                ),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=w style='background:#fcc;min-height:0;height:30px'><div id=a>X<br>X<br>X<br>X<br>X<br>X</div></div><div id=n>N</div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 20.0], [100.0, 0.0, 100.0, 10.0]]),
                (
                    "a",
                    &[
                        [0.0, 0.0, 100.0, 20.0],
                        [100.0, 0.0, 100.0, 20.0],
                        [200.0, 0.0, 100.0, 20.0],
                    ],
                ),
                ("n", &[[100.0, 10.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=w style='background:#fcc;min-height:0;height:30px'><div id=a style='height:20px'></div><div id=b style='height:40px'></div></div><div id=n>N</div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 30.0]]),
                ("a", &[[0.0, 0.0, 100.0, 20.0]]),
                ("b", &[[0.0, 20.0, 100.0, 20.0], [100.0, 0.0, 100.0, 20.0]]),
                ("n", &[[0.0, 30.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=w style='background:#fcc;min-height:0;height:30px'><div id=a style='height:50px;margin-bottom:10px'></div></div><div id=n>N</div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 30.0]]),
                ("a", &[[0.0, 0.0, 100.0, 40.0], [100.0, 0.0, 100.0, 10.0]]),
                ("n", &[[0.0, 30.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=w style='background:#fcc;max-height:30px'><div id=a style='height:60px'></div></div><div id=n>N</div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 30.0]]),
                ("a", &[[0.0, 0.0, 100.0, 40.0], [100.0, 0.0, 100.0, 20.0]]),
                ("n", &[[0.0, 30.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=q>Q</div><div id=w style='background:#fcc;max-height:30px'><div id=a>X<br>X<br>X<br>X<br>X<br>X</div></div><div id=n>N</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 10.0]]),
                ("w", &[[0.0, 10.0, 100.0, 20.0], [100.0, 0.0, 100.0, 10.0]]),
                (
                    "a",
                    &[
                        [0.0, 10.0, 100.0, 20.0],
                        [100.0, 0.0, 100.0, 30.0],
                        [200.0, 0.0, 100.0, 20.0],
                    ],
                ),
                ("n", &[[100.0, 10.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=q>Q</div><div id=w style='background:#fcc;max-height:30px'><div id=a style='height:20px'></div><div id=b style='height:40px'></div></div><div id=n>N</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 10.0]]),
                (
                    "w",
                    &[[0.0, 10.0, 100.0, 13.34375], [100.0, 0.0, 100.0, 16.65625]],
                ),
                (
                    "a",
                    &[[0.0, 10.0, 100.0, 13.34375], [100.0, 0.0, 100.0, 6.65625]],
                ),
                (
                    "b",
                    &[
                        [100.0, 6.65625, 100.0, 16.6875],
                        [200.0, 0.0, 100.0, 23.3125],
                    ],
                ),
                ("n", &[[200.0, 0.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=q>Q</div><div id=w style='background:#fcc;max-height:30px'><div id=a style='height:50px;margin-bottom:10px'></div></div><div id=n>N</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 10.0]]),
                ("w", &[[0.0, 10.0, 100.0, 10.0], [100.0, 0.0, 100.0, 20.0]]),
                (
                    "a",
                    &[
                        [0.0, 10.0, 100.0, 10.0],
                        [100.0, 0.0, 100.0, 20.0],
                        [200.0, 0.0, 100.0, 20.0],
                    ],
                ),
                ("n", &[[200.0, 0.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=q>Q</div><div id=w style='background:#fcc;height:30px;padding-bottom:4px'><div id=a style='height:60px'></div></div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 10.0]]),
                ("w", &[[0.0, 10.0, 100.0, 30.0]]),
                ("a", &[[0.0, 10.0, 100.0, 30.0], [100.0, 0.0, 100.0, 30.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=q>Q</div><div id=w style='background:#fcc;height:30px;padding-bottom:4px'><div id=a style='height:20px'></div><div id=b style='height:40px'></div></div><div id=n>N</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 10.0]]),
                ("w", &[[0.0, 10.0, 100.0, 30.0]]),
                ("a", &[[0.0, 10.0, 100.0, 20.0]]),
                ("b", &[[0.0, 30.0, 100.0, 10.0], [100.0, 0.0, 100.0, 30.0]]),
                ("n", &[[100.0, 0.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=w style='background:#fcc;height:30px;border-top:3px solid'><div id=a style='height:60px'></div></div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 30.0]]),
                ("a", &[[0.0, 3.0, 100.0, 37.0], [100.0, 0.0, 100.0, 23.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=q>Q</div><div id=w style='background:#fcc;height:30px;border-top:3px solid'><div id=a>X<br>X<br>X<br>X<br>X<br>X</div></div><div id=n>N</div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 10.0]]),
                ("w", &[[0.0, 10.0, 100.0, 30.0]]),
                ("a", &[[0.0, 13.0, 100.0, 27.0], [100.0, 0.0, 100.0, 40.0]]),
                ("n", &[[100.0, 0.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=w style='background:#fcc;height:30px;border-top:3px solid'><div id=a style='height:50px;margin-bottom:10px'></div></div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 30.0]]),
                ("a", &[[0.0, 3.0, 100.0, 37.0], [100.0, 0.0, 100.0, 13.0]]),
            ],
        ),
    ];
    for (body, expected) in cases {
        assert_pieces(body, expected);
    }
}

#[test]
fn the_room_a_wrapper_leaves_below_its_boxes_breaks_between_columns() {
    let cases: &[Case] = &[
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=w style='background:#fcc;height:30px'><div id=a style='height:20px'></div></div></div>",
            &[
                (
                    "w",
                    &[
                        [0.0, 0.0, 100.0, 10.0],
                        [100.0, 0.0, 100.0, 10.0],
                        [200.0, 0.0, 100.0, 10.0],
                    ],
                ),
                ("a", &[[0.0, 0.0, 100.0, 10.0], [100.0, 0.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=w style='background:#fcc;height:30px'><div id=a style='height:10px'></div><div id=b style='height:20px'></div></div><div id=n style='height:10px'></div></div>",
            &[
                (
                    "w",
                    &[
                        [0.0, 0.0, 100.0, 13.34375],
                        [100.0, 0.0, 100.0, 13.34375],
                        [200.0, 0.0, 100.0, 3.3125],
                    ],
                ),
                ("a", &[[0.0, 0.0, 100.0, 10.0]]),
                (
                    "b",
                    &[
                        [0.0, 10.0, 100.0, 3.34375],
                        [100.0, 0.0, 100.0, 13.34375],
                        [200.0, 0.0, 100.0, 3.3125],
                    ],
                ),
                ("n", &[[200.0, 3.3125, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=q style='height:10px'></div><div id=w style='background:#fcc;height:30px'><div id=a style='height:30px'></div></div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 10.0]]),
                (
                    "w",
                    &[
                        [0.0, 10.0, 100.0, 3.34375],
                        [100.0, 0.0, 100.0, 13.34375],
                        [200.0, 0.0, 100.0, 13.3125],
                    ],
                ),
                (
                    "a",
                    &[
                        [0.0, 10.0, 100.0, 3.34375],
                        [100.0, 0.0, 100.0, 13.34375],
                        [200.0, 0.0, 100.0, 13.3125],
                    ],
                ),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=w style='background:#fcc;min-height:50px'><div id=a style='height:10px'></div><div id=b style='height:20px'></div></div></div>",
            &[
                (
                    "w",
                    &[
                        [0.0, 0.0, 100.0, 16.671875],
                        [100.0, 0.0, 100.0, 16.671875],
                        [200.0, 0.0, 100.0, 16.65625],
                    ],
                ),
                ("a", &[[0.0, 0.0, 100.0, 10.0]]),
                (
                    "b",
                    &[[0.0, 10.0, 100.0, 6.671875], [100.0, 0.0, 100.0, 13.328125]],
                ),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px'><div id=w style='background:#fcc;min-height:50px'><div id=a style='height:30px'></div></div><div id=n style='height:10px'></div></div>",
            &[
                (
                    "w",
                    &[
                        [0.0, 0.0, 100.0, 20.0],
                        [100.0, 0.0, 100.0, 20.0],
                        [200.0, 0.0, 100.0, 10.0],
                    ],
                ),
                ("a", &[[0.0, 0.0, 100.0, 20.0], [100.0, 0.0, 100.0, 10.0]]),
                ("n", &[[200.0, 10.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=w style='background:#fcc;max-height:30px'><div id=a style='height:30px'></div></div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 30.0]]),
                ("a", &[[0.0, 0.0, 100.0, 30.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=q style='height:10px'></div><div id=w style='background:#fcc;height:30px;padding-bottom:4px'><div id=a style='height:10px'></div><div id=b style='height:20px'></div></div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 10.0]]),
                ("w", &[[0.0, 10.0, 100.0, 30.0]]),
                ("a", &[[0.0, 10.0, 100.0, 10.0]]),
                ("b", &[[0.0, 20.0, 100.0, 20.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=w style='background:#fcc;height:80px'><div id=a style='height:20px'></div></div><div id=n style='height:10px'></div></div>",
            &[
                ("w", &[[0.0, 0.0, 100.0, 40.0], [100.0, 0.0, 100.0, 40.0]]),
                ("a", &[[0.0, 0.0, 100.0, 20.0]]),
                ("n", &[[200.0, 0.0, 100.0, 10.0]]),
            ],
        ),
        (
            "<div id=mc style='columns:3;column-gap:0;width:300px;height:40px;column-fill:auto'><div id=q style='height:10px'></div><div id=w style='background:#fcc;height:80px'><div id=a style='height:10px'></div><div id=b style='height:20px'></div></div></div>",
            &[
                ("q", &[[0.0, 0.0, 100.0, 10.0]]),
                (
                    "w",
                    &[
                        [0.0, 10.0, 100.0, 30.0],
                        [100.0, 0.0, 100.0, 40.0],
                        [200.0, 0.0, 100.0, 10.0],
                    ],
                ),
                ("a", &[[0.0, 10.0, 100.0, 10.0]]),
                ("b", &[[0.0, 20.0, 100.0, 20.0]]),
            ],
        ),
    ];
    for (body, expected) in cases {
        assert_pieces(body, expected);
    }
}
