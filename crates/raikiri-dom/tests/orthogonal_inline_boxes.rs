//! An inline box whose writing mode differs from its parent's is laid out as
//! an `inline-block`: it keeps its own writing mode's axes and takes its size
//! from its content, instead of flowing in the line of its parent.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
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
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

#[test]
fn a_vertical_inline_in_a_horizontal_line_is_laid_out_as_an_inline_block() {
    check(
        r#"<div id="p" style="width:200px">x<span id="s" style="writing-mode:vertical-rl">ab</span>y</div>"#,
        &[
            ("p", (8.0, 8.0, 200.0, 22.0)),
            ("s", (18.0, 8.0, 10.0, 20.0)),
        ],
    );
}

#[test]
fn a_horizontal_inline_in_a_vertical_line_is_laid_out_as_an_inline_block() {
    check(
        r#"<div id="p" style="writing-mode:vertical-lr;height:200px">x<span id="s" style="writing-mode:horizontal-tb">ab</span>y</div>"#,
        &[
            ("p", (8.0, 8.0, 20.0, 200.0)),
            ("s", (8.0, 18.0, 20.0, 10.0)),
        ],
    );
}

#[test]
fn an_inline_in_the_other_vertical_mode_is_laid_out_as_an_inline_block() {
    check(
        r#"<div id="p" style="writing-mode:vertical-rl;height:200px">x<span id="s" style="writing-mode:vertical-lr">ab</span>y</div>"#,
        &[
            ("p", (8.0, 8.0, 10.0, 200.0)),
            ("s", (8.0, 18.0, 10.0, 20.0)),
        ],
    );
}

#[test]
fn a_floated_inline_in_another_writing_mode_is_a_block() {
    check(
        r#"<div id="p" style="width:200px"><span id="s" style="writing-mode:vertical-rl;float:left">ab</span>y</div>"#,
        &[
            ("p", (8.0, 8.0, 200.0, 10.0)),
            ("s", (8.0, 8.0, 10.0, 20.0)),
        ],
    );
}

/// `depth` nested inline boxes that alternate between the writing modes of the
/// golden spiral in `css/css-inline/dominant-baseline`, each holding an `X`
/// and the next box. Box `i` has the id `s{i}`.
fn spiral(depth: usize) -> String {
    const MODES: [&str; 4] = [
        "horizontal-tb",
        "vertical-lr",
        "horizontal-tb;direction:rtl",
        "sideways-rl",
    ];
    let mut body = String::new();
    for i in 0..depth {
        body.push_str(&format!(
            r#"<span id="s{i}" style="writing-mode:{}">X"#,
            MODES[i % MODES.len()]
        ));
    }
    body.push_str(&"</span>".repeat(depth));
    format!(r#"<div id="line">{body}</div>"#)
}

#[test]
fn nested_orthogonal_inline_boxes_match_chromium() {
    check(
        &spiral(4),
        &[
            ("s0", (8.0, 22.0, 30.0, 10.0)),
            ("s1", (18.0, 8.0, 20.0, 22.0)),
            ("s2", (18.0, 18.0, 20.0, 12.0)),
            ("s3", (18.0, 18.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn a_deep_chain_of_orthogonal_inline_boxes_is_laid_out() {
    // Twelve levels laid out the box inside it three times each, and a nested
    // layout had to be redone for every one of those, so the work doubled with
    // every level.
    let boxes = border_boxes(&spiral(12));
    for i in 0..12 {
        assert!(
            boxes.iter().any(|(id, _)| id == &format!("s{i}")),
            "no fragment for #s{i}"
        );
    }
}

/// How many times Taffy dropped the cache entries of a box on laying `body`
/// out, because a result was stored in the other margin-collapsing mode.
///
/// Taffy keeps the entries of one mode per box and drops them all when a result
/// of the other mode is stored, so a box measured in one mode and laid out in
/// the other loses its cache on every probe. The counter is per thread and
/// exists in debug builds only; the layout runs on the calling thread.
#[cfg(debug_assertions)]
fn evictions(body: &str) -> usize {
    let before = evictions_so_far();
    border_boxes(body);
    evictions_so_far() - before
}

#[cfg(debug_assertions)]
fn evictions_so_far() -> usize {
    taffy::cache_mode_change_evictions() // taffy-boundary-lint:ignore: the counter has no owned name; it exists in Taffy only
}

#[cfg(debug_assertions)]
#[test]
fn measuring_an_inline_box_does_not_empty_its_cache() {
    assert_eq!(evictions(&spiral(6)), 0);
}

#[cfg(debug_assertions)]
#[test]
fn measuring_a_float_does_not_empty_its_cache() {
    // The inline-block takes its width from its content, floats included.
    assert_eq!(
        evictions(
            r#"<div style="display:inline-block">a<span style="float:left">b<span style="display:inline-block">c</span></span></div>"#
        ),
        0
    );
}

#[cfg(debug_assertions)]
#[test]
fn measuring_a_block_does_not_empty_its_cache() {
    // A block that establishes a formatting context, and one that does not,
    // in a horizontal and in a vertical root that take their size from their
    // content.
    for style in ["overflow:hidden", "margin-top:5px"] {
        for root in ["", ";writing-mode:vertical-rl"] {
            assert_eq!(
                evictions(&format!(
                    r#"<div style="display:inline-block{root}">x<div style="{style}">y<span style="display:inline-block">z</span></div></div>"#
                )),
                0,
                "{style}{root}"
            );
        }
    }
}
