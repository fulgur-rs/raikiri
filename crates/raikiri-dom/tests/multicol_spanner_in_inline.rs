//! A `column-span: all` block inside an inline box that holds only blocks. The
//! inline box is split around the block and leaves nothing of its own (CSS 2,
//! 9.2.1.1), so the spanner reaches the columns of the container as it does
//! when it is a child of the container.
//!
//! The geometry below is what Chromium lays out for the same documents.

use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The `(x, y, width, height)` of every fragment of the element with the
/// given `id`, in document order.
type Rects = Vec<(f32, f32, f32, f32)>;

/// A box of the given height that stays in one column, with `extra` style.
fn block(id: &str, height: u32, extra: &str) -> String {
    format!("<div id={id} style='height: {height}px; break-inside: avoid; {extra}'></div>")
}

/// A spanner of 20px.
fn spanner() -> &'static str {
    "<div id=s style='column-span: all; height: 20px; background: pink'></div>"
}

/// The fragments of the element `id` in the layout of `body`.
fn rects(body: &str, id: &str) -> Rects {
    let html = format!("<!DOCTYPE html><style>body {{ margin: 0 }}</style>{body}");
    let resources = RenderResources::new();
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

/// Compares the fragments of `id` with the expected ones.
fn assert_rects(body: &str, id: &str, expected: &[(f32, f32, f32, f32)]) {
    let got = rects(body, id);
    let same = got.len() == expected.len()
        && got.iter().zip(expected).all(|(got, expected)| {
            (got.0 - expected.0).abs() <= 0.02
                && (got.1 - expected.1).abs() <= 0.02
                && (got.2 - expected.2).abs() <= 0.02
                && (got.3 - expected.3).abs() <= 0.02
        });
    assert!(same, "#{id}: placed {got:?}, expected {expected:?}");
}

/// A two-column container, 142px wide columns and a 16px gap.
const COLUMNS: &str = "id=a style='columns: 2; width: 300px'";

/// Floats around a spanner that sit in a `span`, as in the WPT test: the
/// spanner goes below the floats above it, which belong to the columns before
/// it, and the floats after it go below the spanner.
#[test]
fn floats_around_a_spanner_in_an_inline_box_stay_in_their_groups() {
    let body = "<style>.l { float: left; width: 100px; height: 20px }\
                .r { float: right; width: 100px; height: 20px }</style>\
                <article id=a style='columns: 1; width: 300px'><span>\
                <div id=f1 class=l></div><div id=f2 class=r></div>\
                <div id=s style='column-span: all; height: 20px'></div>\
                <div id=f3 class=l></div><div id=f4 class=r></div>\
                </span></article>";
    assert_rects(body, "f1", &[(0.0, 0.0, 100.0, 20.0)]);
    assert_rects(body, "f2", &[(200.0, 0.0, 100.0, 20.0)]);
    assert_rects(body, "s", &[(0.0, 20.0, 300.0, 20.0)]);
    assert_rects(body, "f3", &[(0.0, 40.0, 100.0, 20.0)]);
    assert_rects(body, "f4", &[(200.0, 40.0, 100.0, 20.0)]);
    assert_rects(body, "a", &[(0.0, 0.0, 300.0, 60.0)]);
}

/// The blocks before and after the spanner flow in the columns of their own
/// group.
#[test]
fn blocks_around_a_spanner_in_an_inline_box_are_columns_of_their_own() {
    let body = format!(
        "<article {COLUMNS}>{}<span>{}</span>{}</article>",
        block("b1", 20, "background: black"),
        spanner(),
        block("b2", 20, "background: navy")
    );
    assert_rects(&body, "b1", &[(0.0, 0.0, 142.0, 20.0)]);
    assert_rects(&body, "s", &[(0.0, 20.0, 300.0, 20.0)]);
    assert_rects(&body, "b2", &[(0.0, 40.0, 142.0, 20.0)]);
    assert_rects(&body, "a", &[(0.0, 0.0, 300.0, 60.0)]);
}

/// Inline boxes in one another around the spanner leave no box either.
#[test]
fn a_spanner_in_nested_inline_boxes_spans_the_columns() {
    let body = format!(
        "<article {COLUMNS}><span><span>{}</span></span>{}</article>",
        spanner(),
        block("b", 20, "background: black")
    );
    assert_rects(&body, "s", &[(0.0, 0.0, 300.0, 20.0)]);
    assert_rects(&body, "b", &[(0.0, 20.0, 142.0, 20.0)]);
    assert_rects(&body, "a", &[(0.0, 0.0, 300.0, 40.0)]);
}

/// The blocks and the spanner in an inline box are laid out as those of an
/// inline-box-free container.
#[test]
fn an_inline_box_around_blocks_lays_them_out_as_a_container_without_it() {
    let inner = format!(
        "{}{}{}",
        block("b1", 20, "background: black"),
        spanner(),
        block("b2", 20, "background: navy")
    );
    let wrapped = format!("<article {COLUMNS}><span>{inner}</span></article>");
    let bare = format!("<article {COLUMNS}>{inner}</article>");
    for id in ["b1", "s", "b2", "a"] {
        assert_eq!(rects(&wrapped, id), rects(&bare, id), "#{id}");
    }
}
