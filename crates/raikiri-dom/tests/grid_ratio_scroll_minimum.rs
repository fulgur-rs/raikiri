//! A grid item that is a scroll container has an automatic minimum size of
//! zero (CSS Grid 1 §6.6), even when an `aspect-ratio` carries its definite
//! height over to its auto width: its column may be narrower than the item,
//! which then overflows the column at the width the ratio gives. A `clip` or
//! `visible` item is no scroll container and keeps the carried width as its
//! minimum.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport, with the body margin of 8px.

use raikiri_html::{LayoutOptions, LayoutStatus, layout, parse_html};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let doc = parse_html(html.as_bytes(), &Default::default()).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
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

fn check(body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, rect) in expected {
        let found = actual
            .iter()
            .find(|(box_id, _)| box_id == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        assert_eq!(found, rect, "{body}: #{id}");
    }
}

/// A column of auto items in a grid as wide as its minimum content.
fn columns(first: &str, second: &str) -> String {
    format!(
        "<div id=g style='display:grid;width:min-content;grid-auto-flow:column;height:100px'>\
         <div id=a style='{first}'></div><div id=b style='{second}'></div></div>"
    )
}

#[test]
fn a_scroll_container_with_a_carried_width_adds_no_minimum_to_its_column() {
    let both = columns(
        "height:100px;aspect-ratio:2/1;overflow:auto",
        "height:100px;aspect-ratio:1/2;min-width:100px;overflow:auto",
    );
    let expected = [
        ("g", (8.0, 8.0, 100.0, 100.0)),
        ("a", (8.0, 8.0, 200.0, 100.0)),
        ("b", (8.0, 8.0, 100.0, 100.0)),
    ];
    check(&both, &expected);
    for overflow in ["overflow:hidden", "overflow-x:scroll"] {
        let body = columns(
            &format!("height:100px;aspect-ratio:2/1;{overflow}"),
            "height:100px;width:100px",
        );
        check(&body, &expected);
    }
}

#[test]
fn an_item_that_is_no_scroll_container_keeps_its_carried_width() {
    for overflow in ["", "overflow:clip"] {
        let body = columns(
            &format!("height:100px;aspect-ratio:2/1;{overflow}"),
            "height:100px;width:100px",
        );
        check(
            &body,
            &[
                ("g", (8.0, 8.0, 300.0, 100.0)),
                ("a", (8.0, 8.0, 200.0, 100.0)),
                ("b", (208.0, 8.0, 100.0, 100.0)),
            ],
        );
    }
}

#[test]
fn the_items_of_a_scroll_container_that_is_a_grid_are_laid_out_as_before() {
    let body = "<div id=g style='display:grid;width:min-content;grid-auto-flow:column;height:100px'>\
        <div id=a style='display:grid;height:100px;aspect-ratio:2/1;overflow:auto'>\
        <div id=c style='width:30px;height:10px'></div></div>\
        <div id=b style='height:100px;width:100px'></div></div>";
    check(
        body,
        &[
            ("g", (8.0, 8.0, 100.0, 100.0)),
            ("a", (8.0, 8.0, 200.0, 100.0)),
            ("c", (8.0, 8.0, 30.0, 10.0)),
            ("b", (8.0, 8.0, 100.0, 100.0)),
        ],
    );
}

#[test]
fn a_column_with_a_track_minimum_of_auto_also_takes_no_minimum() {
    let body = "<div id=g style='display:grid;width:min-content;grid-template-columns:minmax(auto,50px) auto;height:100px'>\
        <div id=a style='height:100px;aspect-ratio:2/1;overflow:auto'></div>\
        <div id=b style='height:100px;width:100px'></div></div>";
    check(
        body,
        &[
            ("g", (8.0, 8.0, 100.0, 100.0)),
            ("a", (8.0, 8.0, 200.0, 100.0)),
            ("b", (8.0, 8.0, 100.0, 100.0)),
        ],
    );
}

#[test]
fn rows_and_absolute_items_keep_their_sizes() {
    let rows = "<div id=g style='display:grid;width:100px;grid-template-rows:minmax(auto,10px) auto'>\
        <div id=a style='width:100px;aspect-ratio:2/1;overflow:auto'></div>\
        <div id=b style='height:20px'></div></div>";
    check(
        rows,
        &[
            ("g", (8.0, 8.0, 100.0, 30.0)),
            ("a", (8.0, 8.0, 100.0, 50.0)),
            ("b", (8.0, 18.0, 100.0, 20.0)),
        ],
    );
    let absolute = "<div id=g style='display:grid;position:relative;width:min-content;grid-auto-flow:column;height:100px'>\
        <div id=a style='position:absolute;height:100px;aspect-ratio:2/1;overflow:auto'></div>\
        <div id=b style='height:100px;width:100px'></div></div>";
    check(
        absolute,
        &[
            ("g", (8.0, 8.0, 100.0, 100.0)),
            ("a", (8.0, 8.0, 200.0, 100.0)),
            ("b", (8.0, 8.0, 100.0, 100.0)),
        ],
    );
}
