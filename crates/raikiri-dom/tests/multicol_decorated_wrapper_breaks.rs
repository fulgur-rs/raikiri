//! Block-edge decorations on wrappers that a multi-column flow breaks.
//!
//! Each case lays out a wrapper `w` with two children `a` and `b`, followed by
//! a sibling `n`, inside a two-column, auto-filled container. The expected
//! rectangles are what Chromium reports through `getClientRects()` for each
//! fragment, on a page with an 8px body margin.
//!
//! https://drafts.csswg.org/css-break-3/#break-margins
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

/// The left, top, width and height of the element `id` in `body`, on the first
/// page of 800px by 600px.
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
            dom.attr(fragment.node(), "id").map(|id| {
                let rect = fragment.rect();
                (id.to_string(), [rect.x, rect.y, rect.width, rect.height])
            })
        })
        .collect()
}

/// Where the breaks around the wrapper come from.
#[derive(Clone, Copy, Debug)]
enum Variant {
    /// `n` starts a new column.
    Bc,
    /// No break property anywhere.
    None,
    /// `n` avoids a column break before it.
    Avc,
    /// `b` starts a new column inside the wrapper.
    InnerBc,
    /// `a` and `b` are 60px each, so `b` crosses the column end.
    TallBc,
}

fn markup(wrapper: &str, variant: Variant) -> String {
    let (child, b, n) = match variant {
        Variant::Bc => (20, "", "break-before:column"),
        Variant::None => (20, "", ""),
        Variant::Avc => (20, "", "break-before:avoid-column"),
        Variant::InnerBc => (20, "break-before:column", ""),
        Variant::TallBc => (60, "", "break-before:column"),
    };
    format!(
        "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'>\
         <div id=w style='{wrapper}'><div id=a style='height:{child}px'></div>\
         <div id=b style='height:{child}px;{b}'></div></div>\
         <div id=n style='height:20px;{n}'></div></div>"
    )
}

/// The fragments in `got` that differ from the `expected` ones, by id.
fn mismatches(case: &str, got: &[(String, [f32; 4])], expected: Expected) -> Vec<String> {
    let mut failures = Vec::new();
    for &(id, rects) in expected {
        let fragments: Vec<[f32; 4]> = got
            .iter()
            .filter(|(name, _)| name == id)
            .map(|(_, rect)| *rect)
            .collect();
        let matches = fragments.len() == rects.len()
            && fragments
                .iter()
                .zip(rects)
                .all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 0.05));
        if !matches {
            failures.push(format!("{case} #{id}: got {fragments:?}, want {rects:?}"));
        }
    }
    failures
}

type Expected = &'static [(&'static str, &'static [[f32; 4]])];

const CASES: &[(&str, Variant, Expected)] = &[
    (
        "padding-top:10px",
        Variant::Bc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 50.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[8.0, 38.0, 50.0, 20.0]]),
            ("n", &[[58.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-top:10px",
        Variant::None,
        &[
            ("w", &[[8.0, 8.0, 50.0, 50.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[8.0, 38.0, 50.0, 20.0]]),
            ("n", &[[8.0, 58.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-top:10px",
        Variant::Avc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 50.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[8.0, 38.0, 50.0, 20.0]]),
            ("n", &[[8.0, 58.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-top:10px",
        Variant::InnerBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 20.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[58.0, 28.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-top:10px",
        Variant::TallBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 30.0]]),
            ("a", &[[8.0, 18.0, 50.0, 60.0]]),
            ("b", &[[8.0, 78.0, 50.0, 30.0], [58.0, 8.0, 50.0, 30.0]]),
            ("n", &[[108.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-bottom:10px",
        Variant::Bc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 50.0]]),
            ("a", &[[8.0, 8.0, 50.0, 20.0]]),
            ("b", &[[8.0, 28.0, 50.0, 20.0]]),
            ("n", &[[58.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-bottom:10px",
        Variant::None,
        &[
            ("w", &[[8.0, 8.0, 50.0, 50.0]]),
            ("a", &[[8.0, 8.0, 50.0, 20.0]]),
            ("b", &[[8.0, 28.0, 50.0, 20.0]]),
            ("n", &[[8.0, 58.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-bottom:10px",
        Variant::Avc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 50.0]]),
            ("a", &[[8.0, 8.0, 50.0, 20.0]]),
            ("b", &[[8.0, 28.0, 50.0, 20.0]]),
            ("n", &[[8.0, 58.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-bottom:10px",
        Variant::InnerBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 30.0]]),
            ("a", &[[8.0, 8.0, 50.0, 20.0]]),
            ("b", &[[58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[58.0, 38.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-bottom:10px",
        Variant::TallBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 30.0]]),
            ("a", &[[8.0, 8.0, 50.0, 60.0]]),
            ("b", &[[8.0, 68.0, 50.0, 40.0], [58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[108.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-top:30px",
        Variant::Bc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 70.0]]),
            ("a", &[[8.0, 38.0, 50.0, 20.0]]),
            ("b", &[[8.0, 58.0, 50.0, 20.0]]),
            ("n", &[[58.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-top:30px",
        Variant::None,
        &[
            ("w", &[[8.0, 8.0, 50.0, 70.0]]),
            ("a", &[[8.0, 38.0, 50.0, 20.0]]),
            ("b", &[[8.0, 58.0, 50.0, 20.0]]),
            ("n", &[[8.0, 78.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-top:30px",
        Variant::Avc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 70.0]]),
            ("a", &[[8.0, 38.0, 50.0, 20.0]]),
            ("b", &[[8.0, 58.0, 50.0, 20.0]]),
            ("n", &[[8.0, 78.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-top:30px",
        Variant::InnerBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 20.0]]),
            ("a", &[[8.0, 38.0, 50.0, 20.0]]),
            ("b", &[[58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[58.0, 28.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding-top:30px",
        Variant::TallBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 50.0]]),
            ("a", &[[8.0, 38.0, 50.0, 60.0]]),
            ("b", &[[8.0, 98.0, 50.0, 10.0], [58.0, 8.0, 50.0, 50.0]]),
            ("n", &[[108.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border-top:5px solid",
        Variant::Bc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 45.0]]),
            ("a", &[[8.0, 13.0, 50.0, 20.0]]),
            ("b", &[[8.0, 33.0, 50.0, 20.0]]),
            ("n", &[[58.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border-top:5px solid",
        Variant::None,
        &[
            ("w", &[[8.0, 8.0, 50.0, 45.0]]),
            ("a", &[[8.0, 13.0, 50.0, 20.0]]),
            ("b", &[[8.0, 33.0, 50.0, 20.0]]),
            ("n", &[[8.0, 53.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border-top:5px solid",
        Variant::Avc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 45.0]]),
            ("a", &[[8.0, 13.0, 50.0, 20.0]]),
            ("b", &[[8.0, 33.0, 50.0, 20.0]]),
            ("n", &[[8.0, 53.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border-top:5px solid",
        Variant::InnerBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 20.0]]),
            ("a", &[[8.0, 13.0, 50.0, 20.0]]),
            ("b", &[[58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[58.0, 28.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border-top:5px solid",
        Variant::TallBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 25.0]]),
            ("a", &[[8.0, 13.0, 50.0, 60.0]]),
            ("b", &[[8.0, 73.0, 50.0, 35.0], [58.0, 8.0, 50.0, 25.0]]),
            ("n", &[[108.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border:5px solid",
        Variant::Bc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 50.0]]),
            ("a", &[[13.0, 13.0, 40.0, 20.0]]),
            ("b", &[[13.0, 33.0, 40.0, 20.0]]),
            ("n", &[[58.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border:5px solid",
        Variant::None,
        &[
            ("w", &[[8.0, 8.0, 50.0, 50.0]]),
            ("a", &[[13.0, 13.0, 40.0, 20.0]]),
            ("b", &[[13.0, 33.0, 40.0, 20.0]]),
            ("n", &[[8.0, 58.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border:5px solid",
        Variant::Avc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 50.0]]),
            ("a", &[[13.0, 13.0, 40.0, 20.0]]),
            ("b", &[[13.0, 33.0, 40.0, 20.0]]),
            ("n", &[[8.0, 58.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border:5px solid",
        Variant::InnerBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 25.0]]),
            ("a", &[[13.0, 13.0, 40.0, 20.0]]),
            ("b", &[[63.0, 8.0, 40.0, 20.0]]),
            ("n", &[[58.0, 33.0, 50.0, 20.0]]),
        ],
    ),
    (
        "border:5px solid",
        Variant::TallBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 30.0]]),
            ("a", &[[13.0, 13.0, 40.0, 60.0]]),
            ("b", &[[13.0, 73.0, 40.0, 35.0], [63.0, 8.0, 40.0, 25.0]]),
            ("n", &[[108.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "margin-top:10px;margin-bottom:5px",
        Variant::Bc,
        &[
            ("w", &[[8.0, 18.0, 50.0, 40.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[8.0, 38.0, 50.0, 20.0]]),
            ("n", &[[58.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "margin-top:10px;margin-bottom:5px",
        Variant::None,
        &[
            ("w", &[[8.0, 18.0, 50.0, 40.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[8.0, 38.0, 50.0, 20.0]]),
            ("n", &[[8.0, 63.0, 50.0, 20.0]]),
        ],
    ),
    (
        "margin-top:10px;margin-bottom:5px",
        Variant::Avc,
        &[
            ("w", &[[8.0, 18.0, 50.0, 40.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[8.0, 38.0, 50.0, 20.0]]),
            ("n", &[[8.0, 63.0, 50.0, 20.0]]),
        ],
    ),
    (
        "margin-top:10px;margin-bottom:5px",
        Variant::InnerBc,
        &[
            ("w", &[[8.0, 18.0, 50.0, 90.0], [58.0, 8.0, 50.0, 20.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[58.0, 33.0, 50.0, 20.0]]),
        ],
    ),
    (
        "margin-top:10px;margin-bottom:5px",
        Variant::TallBc,
        &[
            ("w", &[[8.0, 18.0, 50.0, 90.0], [58.0, 8.0, 50.0, 30.0]]),
            ("a", &[[8.0, 18.0, 50.0, 60.0]]),
            ("b", &[[8.0, 78.0, 50.0, 30.0], [58.0, 8.0, 50.0, 30.0]]),
            ("n", &[[108.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding:10px 0",
        Variant::Bc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 60.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[8.0, 38.0, 50.0, 20.0]]),
            ("n", &[[58.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding:10px 0",
        Variant::None,
        &[
            ("w", &[[8.0, 8.0, 50.0, 60.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[8.0, 38.0, 50.0, 20.0]]),
            ("n", &[[8.0, 68.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding:10px 0",
        Variant::Avc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 60.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[8.0, 38.0, 50.0, 20.0]]),
            ("n", &[[8.0, 68.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding:10px 0",
        Variant::InnerBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 30.0]]),
            ("a", &[[8.0, 18.0, 50.0, 20.0]]),
            ("b", &[[58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[58.0, 38.0, 50.0, 20.0]]),
        ],
    ),
    (
        "padding:10px 0",
        Variant::TallBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 40.0]]),
            ("a", &[[8.0, 18.0, 50.0, 60.0]]),
            ("b", &[[8.0, 78.0, 50.0, 30.0], [58.0, 8.0, 50.0, 30.0]]),
            ("n", &[[108.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "",
        Variant::Bc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 40.0]]),
            ("a", &[[8.0, 8.0, 50.0, 20.0]]),
            ("b", &[[8.0, 28.0, 50.0, 20.0]]),
            ("n", &[[58.0, 8.0, 50.0, 20.0]]),
        ],
    ),
    (
        "",
        Variant::None,
        &[
            ("w", &[[8.0, 8.0, 50.0, 40.0]]),
            ("a", &[[8.0, 8.0, 50.0, 20.0]]),
            ("b", &[[8.0, 28.0, 50.0, 20.0]]),
            ("n", &[[8.0, 48.0, 50.0, 20.0]]),
        ],
    ),
    (
        "",
        Variant::Avc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 40.0]]),
            ("a", &[[8.0, 8.0, 50.0, 20.0]]),
            ("b", &[[8.0, 28.0, 50.0, 20.0]]),
            ("n", &[[8.0, 48.0, 50.0, 20.0]]),
        ],
    ),
    (
        "",
        Variant::InnerBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 20.0]]),
            ("a", &[[8.0, 8.0, 50.0, 20.0]]),
            ("b", &[[58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[58.0, 28.0, 50.0, 20.0]]),
        ],
    ),
    (
        "",
        Variant::TallBc,
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 20.0]]),
            ("a", &[[8.0, 8.0, 50.0, 60.0]]),
            ("b", &[[8.0, 68.0, 50.0, 40.0], [58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[108.0, 8.0, 50.0, 20.0]]),
        ],
    ),
];

#[test]
fn decorated_wrappers_break_between_columns_like_chromium() {
    let mut failures = Vec::new();
    for &(wrapper, variant, expected) in CASES {
        let got = boxes(&markup(wrapper, variant));
        failures.extend(mismatches(
            &format!("{wrapper:?} {variant:?}"),
            &got,
            expected,
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Children of the same two-column container, for the margins and edges next
/// to the end of a column and around empty boxes.
const EDGE_CASES: &[(&str, Expected)] = &[
    (
        "<div style='height:100px'></div><div id=w><div id=a style='height:20px'></div></div><div id=n style='height:20px;break-before:avoid'></div>",
        &[
            ("w", &[[58.0, 8.0, 50.0, 20.0]]),
            ("a", &[[58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[58.0, 28.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div style='height:100px'></div><div id=w style='margin-top:10px'><div id=a style='height:20px'></div></div><div id=n style='height:20px;break-before:avoid'></div>",
        &[
            ("w", &[[58.0, 8.0, 50.0, 20.0]]),
            ("a", &[[58.0, 8.0, 50.0, 20.0]]),
            ("n", &[[58.0, 28.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div style='height:100px'></div><div id=w style='border-top:5px solid'><div id=a style='height:20px'></div></div><div id=n style='height:20px;break-before:avoid'></div>",
        &[
            ("w", &[[58.0, 8.0, 50.0, 25.0]]),
            ("a", &[[58.0, 13.0, 50.0, 20.0]]),
            ("n", &[[58.0, 33.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div style='height:95px'></div><div id=w style='border-top:5px solid'><div id=a style='height:20px'></div></div><div id=n style='height:20px;break-before:avoid'></div>",
        &[
            ("w", &[[58.0, 8.0, 50.0, 25.0]]),
            ("a", &[[58.0, 13.0, 50.0, 20.0]]),
            ("n", &[[58.0, 33.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div id=p style='height:30px'></div><div id=w style='margin-bottom:30px'><div id=a style='height:30px'></div></div><div id=n style='height:30px;break-before:avoid;break-inside:avoid'></div>",
        &[
            ("p", &[[8.0, 8.0, 50.0, 30.0]]),
            ("w", &[[58.0, 8.0, 50.0, 30.0]]),
            ("a", &[[58.0, 8.0, 50.0, 30.0]]),
            ("n", &[[58.0, 68.0, 50.0, 30.0]]),
        ],
    ),
    (
        "<div id=w style='padding-bottom:10px'><div id=a style='height:50px'></div><div id=b style='height:50px;break-inside:avoid'></div></div><div id=n style='height:20px;break-before:avoid'></div>",
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 60.0]]),
            ("a", &[[8.0, 8.0, 50.0, 50.0]]),
            ("b", &[[58.0, 8.0, 50.0, 50.0]]),
            ("n", &[[58.0, 68.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div id=w style='padding-bottom:10px'><div id=a style='height:50px'></div><div id=b style='height:50px'></div></div><div id=n style='height:20px;break-before:avoid'></div>",
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 60.0]]),
            ("a", &[[8.0, 8.0, 50.0, 50.0]]),
            ("b", &[[58.0, 8.0, 50.0, 50.0]]),
            ("n", &[[58.0, 68.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div id=w style='padding-bottom:10px'><div id=a style='height:45px'></div><div id=b style='height:50px;break-inside:avoid'></div></div><div id=n style='height:20px;break-before:avoid'></div>",
        &[
            ("w", &[[8.0, 8.0, 50.0, 100.0], [58.0, 8.0, 50.0, 60.0]]),
            ("a", &[[8.0, 8.0, 50.0, 45.0]]),
            ("b", &[[58.0, 8.0, 50.0, 50.0]]),
            ("n", &[[58.0, 68.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div style='height:20px'></div><div id=w style='margin-bottom:20px'><div id=a style='break-before:column'></div></div><div id=n style='height:20px'></div>",
        &[
            ("w", &[[58.0, 8.0, 50.0, 0.0]]),
            ("a", &[[58.0, 8.0, 50.0, 0.0]]),
            ("n", &[[58.0, 28.0, 50.0, 20.0]]),
        ],
    ),
    (
        "<div id=w style='margin-bottom:20px'><div style='height:10px'></div></div><div id=e></div><div id=v style='margin-top:30px'><div id=a style='height:10px'></div></div><div id=n style='height:10px;break-before:avoid'></div>",
        &[
            ("w", &[[8.0, 8.0, 50.0, 10.0]]),
            ("e", &[[8.0, 38.0, 50.0, 0.0]]),
            ("v", &[[8.0, 48.0, 50.0, 10.0]]),
            ("a", &[[8.0, 48.0, 50.0, 10.0]]),
            ("n", &[[8.0, 58.0, 50.0, 10.0]]),
        ],
    ),
];

#[test]
fn wrapper_edges_at_column_ends_and_empty_boxes_match_chromium() {
    let mut failures = Vec::new();
    for &(children, expected) in EDGE_CASES {
        let got = boxes(&format!(
            "<div style='columns:2;column-fill:auto;gap:0;width:100px;height:100px'>{children}</div>"
        ));
        failures.extend(mismatches(children, &got, expected));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
