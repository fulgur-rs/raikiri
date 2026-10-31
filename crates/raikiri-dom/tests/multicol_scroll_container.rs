//! A scroll container with a block size of its own in the columns of a
//! multicol container: it is monolithic, so a column end never cuts it. It
//! sits whole in one column, moves to the next one when it does not fit below
//! what comes before it, and overflows the column it starts at the top of when
//! it is taller than the column (CSS Fragmentation 3, monolithic content).
//!
//! The geometry below is what Chromium lays out for the same documents.

use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The `(x, y, width, height)` of every fragment of the element with the
/// given `id`, in document order.
type Rects = Vec<(f32, f32, f32, f32)>;

/// What a scroll container holds: a box much larger than the container itself.
const CONTENT: &str = "<div style='width: 400px; height: 400px; background: green'></div>";

/// A two-column container, 100px wide columns, filled in turn, 100px high.
const COLUMNS: &str =
    "id=m style='columns: 2; width: 200px; height: 100px; column-gap: 0; column-fill: auto'";

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

/// A scroll container of the given height, with `extra` style.
fn scroller(id: &str, height: u32, extra: &str) -> String {
    format!(
        "<div id={id} style='overflow: scroll; background: red; height: {height}px; {extra}'>\
         {CONTENT}</div>"
    )
}

/// A box of the given height that holds nothing.
fn spacer(id: &str, height: u32, extra: &str) -> String {
    format!("<div id={id} style='height: {height}px; {extra}'></div>")
}

/// A scroll container taller than a column that starts the first one stays in
/// it and overflows it: it is not cut at the column end.
#[test]
fn a_scroll_container_taller_than_a_column_overflows_the_column_it_starts() {
    let body = format!("<div {COLUMNS}>{}</div>", scroller("s", 150, ""));
    assert_rects(&body, "s", &[(0.0, 0.0, 100.0, 150.0)]);
    assert_rects(&body, "m", &[(0.0, 0.0, 200.0, 100.0)]);
}

/// A scroll container after a full column starts the next one, and is not cut
/// there either.
#[test]
fn a_scroll_container_after_a_full_column_starts_the_next_column() {
    let body = format!(
        "<div {COLUMNS}>{}{}</div>",
        spacer("e", 100, ""),
        scroller("s", 150, "")
    );
    assert_rects(&body, "e", &[(0.0, 0.0, 100.0, 100.0)]);
    assert_rects(&body, "s", &[(100.0, 0.0, 100.0, 150.0)]);
}

/// A scroll container that fits below what comes before it stays in the
/// column, and one that does not moves to the next column whole.
#[test]
fn a_scroll_container_moves_to_the_next_column_only_when_it_does_not_fit() {
    let fits = format!(
        "<div {COLUMNS}>{}{}</div>",
        spacer("e", 40, ""),
        scroller("s", 40, "")
    );
    assert_rects(&fits, "s", &[(0.0, 40.0, 100.0, 40.0)]);
    let does_not_fit = format!(
        "<div {COLUMNS}>{}{}</div>",
        spacer("e", 60, ""),
        scroller("s", 60, "")
    );
    assert_rects(&does_not_fit, "s", &[(100.0, 0.0, 100.0, 60.0)]);
}

/// Scroll containers one after the other fill a column until the next one
/// does not fit.
#[test]
fn scroll_containers_fill_a_column_in_turn() {
    let body = format!(
        "<div id=m style='columns: 2; width: 200px; height: 100px; column-gap: 0'>{}{}{}</div>",
        scroller("s1", 50, ""),
        scroller("s2", 50, ""),
        scroller("s3", 50, "")
    );
    assert_rects(&body, "s1", &[(0.0, 0.0, 100.0, 50.0)]);
    assert_rects(&body, "s2", &[(0.0, 50.0, 100.0, 50.0)]);
    assert_rects(&body, "s3", &[(100.0, 0.0, 100.0, 50.0)]);
}

/// The margin above a scroll container collapses with the one above it where
/// they meet in a column, and is dropped where they meet at a break.
#[test]
fn the_margin_above_a_scroll_container_is_dropped_at_a_break() {
    let in_column = format!(
        "<div {COLUMNS}>{}{}</div>",
        spacer("e", 40, "margin-bottom: 10px"),
        scroller("s", 40, "margin-top: 20px")
    );
    assert_rects(&in_column, "s", &[(0.0, 60.0, 100.0, 40.0)]);
    let at_break = format!(
        "<div {COLUMNS}>{}{}</div>",
        spacer("e", 70, ""),
        scroller("s", 40, "margin-top: 10px")
    );
    assert_rects(&at_break, "s", &[(100.0, 0.0, 100.0, 40.0)]);
}

/// The padding and border of a scroll container are no place to cut it: the
/// box keeps its full height in its column, and what follows it fills the
/// rest of the column and goes on in the next.
#[test]
fn the_padding_and_border_of_a_scroll_container_are_kept_whole() {
    let body = format!(
        "<div {COLUMNS}>\
         <div id=s style='overflow: hidden; height: 40px; padding: 5px; border: 3px solid; \
         background: red'>{CONTENT}</div>{}</div>",
        spacer("e", 60, "")
    );
    assert_rects(&body, "s", &[(0.0, 0.0, 100.0, 56.0)]);
    assert_rects(
        &body,
        "e",
        &[(0.0, 56.0, 100.0, 44.0), (100.0, 0.0, 100.0, 16.0)],
    );
}

/// Every value of `overflow` that makes a scroll container makes a monolithic
/// box, on one axis or on both.
#[test]
fn every_scrolling_overflow_value_makes_a_monolithic_box() {
    for overflow in [
        "overflow: hidden",
        "overflow: auto",
        "overflow: scroll",
        "overflow-x: hidden",
        "overflow-y: scroll",
    ] {
        let body = format!(
            "<div {COLUMNS}>{}<div id=s style='{overflow}; height: 150px'>{CONTENT}</div></div>",
            spacer("e", 100, "")
        );
        assert_rects(&body, "s", &[(100.0, 0.0, 100.0, 150.0)]);
    }
}

/// Boxes after a scroll container go on in its column and cut as they would
/// without it.
#[test]
fn boxes_after_a_scroll_container_continue_in_its_column() {
    let body = format!(
        "<div id=m style='columns: 2; width: 200px; height: 100px; column-gap: 0'>{}\
         <div id=s style='overflow: auto; height: 80px; background: red'>{CONTENT}</div>{}</div>",
        spacer("e", 30, ""),
        spacer("f", 30, "")
    );
    assert_rects(&body, "e", &[(0.0, 0.0, 100.0, 30.0)]);
    assert_rects(&body, "s", &[(100.0, 0.0, 100.0, 80.0)]);
    assert_rects(
        &body,
        "f",
        &[(100.0, 80.0, 100.0, 20.0), (200.0, 0.0, 100.0, 10.0)],
    );
    let in_turn = format!(
        "<div {COLUMNS}>{}\
         <div id=s style='overflow: hidden; height: 30px; background: red'>{CONTENT}</div>{}</div>",
        spacer("e", 90, ""),
        spacer("f", 30, "")
    );
    assert_rects(&in_turn, "s", &[(100.0, 0.0, 100.0, 30.0)]);
    assert_rects(&in_turn, "f", &[(100.0, 30.0, 100.0, 30.0)]);
}

/// The columns of a container with an auto height are as tall as its tallest
/// scroll container, and what follows the container starts below them.
#[test]
fn balanced_columns_are_as_tall_as_the_tallest_scroll_container() {
    let two = format!(
        "<div id=m style='columns: 2; width: 200px; column-gap: 0'>{}{}</div>\
         <div id=after style='width: 50px; height: 10px'></div>",
        scroller("s1", 60, ""),
        scroller("s2", 60, "")
    );
    assert_rects(&two, "m", &[(0.0, 0.0, 200.0, 60.0)]);
    assert_rects(&two, "s1", &[(0.0, 0.0, 100.0, 60.0)]);
    assert_rects(&two, "s2", &[(100.0, 0.0, 100.0, 60.0)]);
    assert_rects(&two, "after", &[(0.0, 60.0, 50.0, 10.0)]);
    let tall = format!(
        "<div id=m style='columns: 2; width: 200px; column-gap: 0'>{}{}</div>\
         <div id=after style='width: 50px; height: 10px'></div>",
        scroller("s", 150, ""),
        spacer("e", 50, "")
    );
    assert_rects(&tall, "m", &[(0.0, 0.0, 200.0, 150.0)]);
    assert_rects(&tall, "s", &[(0.0, 0.0, 100.0, 150.0)]);
    assert_rects(&tall, "e", &[(100.0, 0.0, 100.0, 50.0)]);
    assert_rects(&tall, "after", &[(0.0, 150.0, 50.0, 10.0)]);
}

/// The page, from 0, and the `(x, y, width, height)` on that page's area of
/// every fragment of the element `id`, with 48px page margins and pages of
/// 480px by 288px.
fn paged(body: &str, id: &str) -> Vec<(usize, f32, f32, f32, f32)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 480px 288px; margin: 48px }} \
         body {{ margin: 0 }}</style>{body}"
    );
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
    for (index, page) in result.pages().enumerate() {
        let dom = page.dom();
        for fragment in page.fragments() {
            if dom.attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                found.push((index, rect.x - 48.0, rect.y - 48.0, rect.width, rect.height));
            }
        }
    }
    found
}

/// The pages fill with scroll containers column after column, each one whole.
#[test]
fn scroll_containers_fill_the_columns_of_each_page_in_turn() {
    let body = format!(
        "<div style='columns: 2; column-gap: 0; column-fill: auto'>{}{}{}</div>",
        scroller("a", 150, ""),
        scroller("b", 150, ""),
        scroller("c", 150, "")
    );
    assert_eq!(paged(&body, "a"), [(0, 0.0, 0.0, 192.0, 150.0)]);
    assert_eq!(paged(&body, "b"), [(0, 192.0, 0.0, 192.0, 150.0)]);
    assert_eq!(paged(&body, "c"), [(1, 0.0, 0.0, 192.0, 150.0)]);
}

/// A scroll container taller than the page it lands on is cut by the pages,
/// as it is without columns: it would otherwise run past the page edge with
/// nothing to carry the rest of it on. The container keeps the single column
/// that the pages cut, so the second box goes on below the first.
#[test]
fn a_scroll_container_taller_than_a_page_goes_on_over_the_pages() {
    let body = format!(
        "<div style='columns: 2; column-gap: 0; column-fill: auto'>{}{}</div>",
        scroller("a", 300, ""),
        scroller("b", 300, "")
    );
    assert_eq!(
        paged(&body, "a"),
        [(0, 0.0, 0.0, 192.0, 192.0), (1, 0.0, 0.0, 192.0, 108.0)]
    );
    assert_eq!(paged(&body, "b")[0], (1, 0.0, 108.0, 192.0, 84.0));
}

/// A margin that leaves a scroll container no room in the column it would
/// start moves it to the next column, where the margin is dropped: only a box
/// at the very top of a column may overflow it.
#[test]
fn a_margin_that_pushes_a_scroll_container_out_of_a_column_moves_it_on() {
    let tall = format!(
        "<div {COLUMNS}>{}</div>",
        scroller("s", 150, "margin-top: 30px")
    );
    assert_rects(&tall, "s", &[(100.0, 0.0, 100.0, 150.0)]);
    let short = format!(
        "<div {COLUMNS}>{}</div>",
        scroller("s", 60, "margin-top: 50px")
    );
    assert_rects(&short, "s", &[(100.0, 0.0, 100.0, 60.0)]);
}

/// A scroll container that holds a box that could reach out of it, or a
/// column container, is left to the other strategies, which keep its height.
#[test]
fn a_scroll_container_holding_boxes_that_could_reach_out_keeps_the_other_strategies() {
    for inside in [
        "<div style='position: absolute; width: 10px; height: 10px'></div>",
        "<div style='columns: 2; height: 50px'><div style='height: 20px'></div></div>",
    ] {
        let body = format!(
            "<div {COLUMNS}><div id=s style='overflow: auto; height: 150px'>{inside}</div></div>"
        );
        let placed = rects(&body, "s");
        assert!(
            placed.len() == 1 && (placed[0].3 - 150.0).abs() <= 0.02,
            "#s: placed {placed:?}"
        );
    }
}

/// A scroll container that is positioned out of the flow, or sticks to its
/// scrollport, is left to the other strategies too, which keep its height.
#[test]
fn a_positioned_scroll_container_keeps_the_other_strategies() {
    for position in ["absolute", "sticky"] {
        let body = format!(
            "<div {COLUMNS}>{}</div>",
            scroller("s", 150, &format!("position: {position}"))
        );
        let placed = rects(&body, "s");
        assert!(
            placed.len() == 1 && (placed[0].3 - 150.0).abs() <= 0.02,
            "#s ({position}): placed {placed:?}"
        );
    }
}
