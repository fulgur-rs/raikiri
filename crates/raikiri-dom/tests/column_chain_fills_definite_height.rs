//! A paragraph inside plain wrappers that needs all of the definite height of
//! its multicolumn container fills the columns in turn and goes on in overflow
//! columns beside them (CSS Multi-column 1, section 8.2). The lines stay
//! inside the container's box, so in print the container and the boxes after
//! it take no more of the page than that box does, and no page is added for
//! the lines of the later columns.
//!
//! The expected pages and line positions are the ones Chromium prints for the
//! same markup on a 180px by 160px page with no margin and 20px Ahem text.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// `count` lines, each in a span with the id `l0`, `l1`, and so on.
fn spans(count: usize) -> String {
    (0..count)
        .map(|index| format!("<span id=l{index}>X</span>"))
        .collect::<Vec<_>>()
        .join("<br>")
}

/// The number of pages, and the left and top of the first piece of each
/// element with an `id`.
fn lay_out(body: &str) -> (u32, Vec<(String, u32, f32, f32)>) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 180px 160px; margin: 0 }} \
         body {{ margin: 0; font: 20px/20px Ahem }} p {{ margin: 0 }} \
         .mc {{ width: 100px; column-count: 2; column-gap: 20px; height: 100px; \
         orphans: 1; widows: 1 }}</style>{body}"
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
    let mut pieces: Vec<(String, u32, f32, f32)> = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("page");
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            if pieces.iter().all(|(seen, ..)| seen != id) {
                let rect = fragment.rect();
                pieces.push((id.to_owned(), index, rect.x, rect.y));
            }
        }
    }
    (result.page_count(), pieces)
}

/// Checks that `pieces` puts the element `id` on the first page at `left`
/// and `top`.
fn assert_at(label: &str, pieces: &[(String, u32, f32, f32)], id: &str, left: f32, top: f32) {
    let (_, page, x, y) = pieces
        .iter()
        .find(|(name, ..)| name == id)
        .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
    assert!(
        *page == 0 && (*x - left).abs() <= 0.05 && (*y - top).abs() <= 0.05,
        "{label}: #{id} on page {page} at {x},{y}, expected {left},{top}"
    );
}

/// Lays out `body` and checks that it prints on one page, with as many lines
/// in each column as `columns` gives, each column 60px right of the one
/// before, and the box `after`, if given, at that top.
fn check(label: &str, body: &str, columns: &[usize], after: Option<f32>) {
    let (pages, pieces) = lay_out(body);
    assert_eq!(pages, 1, "{label}: pages, pieces {pieces:?}");
    let mut line = 0;
    for (column, &count) in columns.iter().enumerate() {
        for row in 0..count {
            let left = 60.0 * column as f32;
            assert_at(label, &pieces, &format!("l{line}"), left, 20.0 * row as f32);
            line += 1;
        }
    }
    if let Some(top) = after {
        assert_at(label, &pieces, "after", 0.0, top);
    }
}

#[test]
fn a_wrapped_paragraph_that_fills_the_definite_height_prints_on_one_page() {
    check(
        "ten_lines",
        &format!("<div class=mc><div><p>{}</p></div></div>", spans(10)),
        &[5, 5],
        None,
    );
    check(
        "nine_lines",
        &format!(
            "<div class=mc><div><div><p>{}</p></div></div></div>",
            spans(9)
        ),
        &[5, 4],
        None,
    );
    check(
        "taller_container",
        &format!(
            "<div class=mc style=\"height:140px\"><div><p>{}</p></div></div>",
            spans(14)
        ),
        &[7, 7],
        None,
    );
}

#[test]
fn lines_past_the_last_column_go_on_beside_it_and_the_flow_continues_below_the_box() {
    check(
        "twelve_lines",
        &format!(
            "<div class=mc><div><p>{}</p></div></div><div id=after style=\"height:20px\"></div>",
            spans(12)
        ),
        &[5, 5, 2],
        Some(100.0),
    );
}

#[test]
fn the_default_widows_move_lines_into_the_last_overflow_column() {
    check(
        "eleven_lines",
        &format!(
            "<div class=mc style=\"orphans:2;widows:2\"><div><p>{}</p></div></div>",
            spans(11)
        ),
        &[5, 4, 2],
        None,
    );
}

#[test]
fn a_line_taller_than_the_columns_starts_the_next_column_on_the_same_page() {
    let label = "tall_line";
    let (pages, pieces) = lay_out(&format!(
        "<div class=mc><div><p>{}<br><span id=t style=\"display:inline-block;\
         width:20px;height:120px\"></span></p></div></div><div id=after style=\"height:20px\"></div>",
        spans(3)
    ));
    assert_eq!(pages, 1, "{label}: pages, pieces {pieces:?}");
    for line in 0..3 {
        assert_at(label, &pieces, &format!("l{line}"), 0.0, 20.0 * line as f32);
    }
    assert_at(label, &pieces, "t", 60.0, 0.0);
    assert_at(label, &pieces, "after", 0.0, 100.0);
}
