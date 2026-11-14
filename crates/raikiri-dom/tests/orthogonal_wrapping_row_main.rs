//! A wrapping row flex container breaks its lines against its main size, its
//! inline size, once its parent has settled it (CSS Flexbox 1 §9.3). A parent
//! that sizes the container from its content, as a flex container sizes an
//! item whose writing mode is orthogonal to its own, still settles that size:
//! the items wrap against it within the container's `max-*` size. The expected
//! boxes are the ones Chromium lays out for the same markup on an 800x600
//! page.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::HashMap;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border box `(x, y, width, height)` of every element with an `id` on the
/// first page of an 800x600 document that holds `body`.
fn boxes(body: &str) -> HashMap<String, [f32; 4]> {
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let mut found = HashMap::new();
    for fragment in page.fragments() {
        if let Some(id) = dom.attr(fragment.node(), "id") {
            let rect = fragment.rect();
            found.insert(id.to_string(), [rect.x, rect.y, rect.width, rect.height]);
        }
    }
    found
}

/// One case: the markup of the body and the expected border box of each
/// element by its `id`.
type Case<'a> = (&'a str, &'a [(&'a str, [f32; 4])]);

fn assert_cases(cases: &[Case<'_>]) {
    let mut failures = Vec::new();
    for (body, expected) in cases {
        let found = boxes(body);
        for (id, want) in *expected {
            let got = found.get(*id).copied();
            let matches = got.is_some_and(|got| {
                got.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= 0.02)
            });
            if !matches {
                failures.push(format!("{body}: {id} expected {want:?}, got {got:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_orthogonal_wrapping_row_breaks_its_lines_against_the_main_size_it_is_given() {
    // A vertical row in a horizontal flex container wraps within its
    // `max-height`, and a horizontal row in a vertical flex container within
    // its `max-width` or the width the content gives it.
    assert_cases(&[
        (
            "<div style=\"writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px\"><div id=c style=\"writing-mode:horizontal-tb;display:flex;flex-wrap:wrap;max-width:100px;\"><div id=a style=\"flex:1 0 40px\">a</div><div id=b style=\"flex:1 0 50px\">b</div><div id=d style=\"flex:0 0 30px\">d</div></div></div>",
            &[
                ("c", [200.0, 0.0, 100.0, 20.0]),
                ("a", [200.0, 0.0, 45.0, 10.0]),
                ("b", [245.0, 0.0, 55.0, 10.0]),
                ("d", [200.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style=\"writing-mode:vertical-rl;display:flex;align-items:flex-start;width:300px;height:200px\"><div id=c style=\"writing-mode:horizontal-tb;display:inline-flex;flex-wrap:wrap;max-width:100px;\"><div id=a style=\"flex:1 0 40px\">a</div><div id=b style=\"flex:1 0 50px\">b</div><div id=d style=\"flex:0 0 30px\">d</div></div></div>",
            &[
                ("c", [200.0, 0.0, 100.0, 20.0]),
                ("a", [200.0, 0.0, 45.0, 10.0]),
                ("b", [245.0, 0.0, 55.0, 10.0]),
                ("d", [200.0, 10.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style=\"display:flex;align-items:flex-start;width:300px\"><div id=c style=\"writing-mode:vertical-rl;display:flex;flex-wrap:wrap;max-height:100px\"><div id=a style=\"flex:1 0 40px\">a</div><div id=b style=\"flex:1 0 50px\">b</div><div id=d style=\"flex:0 0 30px\">d</div></div></div>",
            &[
                ("c", [0.0, 0.0, 20.0, 100.0]),
                ("a", [10.0, 0.0, 10.0, 45.0]),
                ("b", [10.0, 45.0, 10.0, 55.0]),
                ("d", [0.0, 0.0, 10.0, 30.0]),
            ],
        ),
        (
            "<div style=\"display:flex;align-items:flex-start;width:300px\"><div id=c style=\"writing-mode:vertical-rl;display:flex;flex-wrap:wrap\"><div id=a>xxxx xxxx</div><div id=b>xx xx xx xx xx</div><div id=d style=\"flex:1 1 20%\">x</div></div></div>",
            &[
                ("c", [0.0, 0.0, 20.0, 240.0]),
                ("a", [10.0, 0.0, 10.0, 90.0]),
                ("b", [10.0, 90.0, 10.0, 140.0]),
                ("d", [0.0, 0.0, 10.0, 240.0]),
            ],
        ),
        (
            "<div style=\"display:flex;flex-direction:column;align-items:flex-start;width:300px\"><div id=c style=\"writing-mode:vertical-rl;display:inline-flex;flex-wrap:wrap\"><div id=a>xxxx xxxx</div><div id=b>xx xx xx xx xx</div><div id=d style=\"flex:1 1 20%\">x</div></div></div>",
            &[
                ("c", [0.0, 0.0, 20.0, 240.0]),
                ("a", [10.0, 0.0, 10.0, 90.0]),
                ("b", [10.0, 90.0, 10.0, 140.0]),
                ("d", [0.0, 0.0, 10.0, 240.0]),
            ],
        ),
        (
            "<div style=\"display:flex;align-items:flex-start;width:300px\"><div id=c style=\"writing-mode:vertical-rl;display:inline-flex;flex-wrap:wrap-reverse;max-height:100px\"><div id=a style=\"flex:1 0 40px\">a</div><div id=b style=\"flex:1 0 50px\">b</div><div id=d style=\"flex:0 0 30px\">d</div></div></div>",
            &[
                ("c", [0.0, 0.0, 20.0, 100.0]),
                ("a", [0.0, 0.0, 10.0, 45.0]),
                ("b", [0.0, 45.0, 10.0, 55.0]),
                ("d", [10.0, 0.0, 10.0, 30.0]),
            ],
        ),
        (
            "<div style=\"display:flex;align-items:flex-start;width:300px;height:200px\"><div id=c style=\"writing-mode:vertical-rl;display:inline-flex;flex-wrap:wrap;box-sizing:border-box;max-height:100px;padding:30px 0\"><div id=a style=\"writing-mode:horizontal-tb;flex:1 0 40px\">a</div><div id=b style=\"writing-mode:horizontal-tb;flex:1 0 50px\">b</div><div id=d style=\"writing-mode:horizontal-tb;flex:0 0 30px\">d</div></div></div>",
            &[
                ("c", [0.0, 0.0, 30.0, 100.0]),
                ("a", [20.0, 30.0, 10.0, 40.0]),
                ("b", [10.0, 30.0, 10.0, 50.0]),
                ("d", [0.0, 30.0, 10.0, 30.0]),
            ],
        ),
    ]);
}
