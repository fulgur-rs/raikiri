//! Cells placed straight inside a table form anonymous rows in source order, so
//! a row group between them splits the run into separate rows (CSS 2 §17.2.1).
//! The first header group is then drawn above every row and the first footer
//! group below every row (CSS 2 §17.2), the other groups keep their place.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px: each is an element's
//! left, top, width and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of each element in `ids`, on the first page
/// of 800px by 600px.
fn border_boxes(body: &str, ids: &[&str]) -> Vec<[f32; 4]> {
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
    ids.iter()
        .map(|id| {
            page.fragments()
                .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
                .map(|fragment| {
                    let rect = fragment.rect();
                    [rect.x, rect.y, rect.width, rect.height]
                })
                .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

fn cell(id: &str, text: &str) -> String {
    format!("<div id={id} style=\"display:table-cell\">{text}</div>")
}

fn group(kind: &str, id: &str, inner: &str) -> String {
    format!("<div id={id} style=\"display:table-{kind}\">{inner}</div>")
}

fn table(inner: &str) -> String {
    format!("<div id=t style=\"display:table\">{inner}</div>")
}

/// Checks the boxes of the elements in `expected`, given as an id and the left,
/// top, width and height Chromium gives it.
fn check(label: &str, body: &str, expected: &[(&str, [f32; 4])]) {
    let ids: Vec<&str> = expected.iter().map(|&(id, _)| id).collect();
    let found = border_boxes(body, &ids);
    for ((id, want), got) in expected.iter().zip(found) {
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() <= TOLERANCE),
            "{label}: #{id} is {got:?}, expected {want:?} (left, top, width, height)"
        );
    }
}

#[test]
fn bare_cells_on_both_sides_of_a_header_group_form_two_rows() {
    let body = table(&format!(
        "{}{}{}{}{}",
        cell("a", "a"),
        cell("b", "b"),
        group("header-group", "H", &cell("h", "h")),
        cell("e", "e"),
        cell("f", "f"),
    ));
    check(
        "header between cell pairs",
        &body,
        &[
            ("t", [8.0, 8.0, 20.0, 30.0]),
            ("H", [8.0, 8.0, 20.0, 10.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [18.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
            ("f", [18.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn bare_cells_on_both_sides_of_a_footer_group_form_two_rows() {
    let body = table(&format!(
        "{}{}{}{}{}",
        cell("a", "a"),
        cell("b", "b"),
        group("footer-group", "F", &cell("g", "g")),
        cell("e", "e"),
        cell("f", "f"),
    ));
    check(
        "footer between cell pairs",
        &body,
        &[
            ("t", [8.0, 8.0, 20.0, 30.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("b", [18.0, 8.0, 10.0, 10.0]),
            ("e", [8.0, 18.0, 10.0, 10.0]),
            ("f", [18.0, 18.0, 10.0, 10.0]),
            ("F", [8.0, 28.0, 20.0, 10.0]),
            ("g", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn a_header_and_a_footer_group_each_split_the_cells_around_them() {
    let body = table(&format!(
        "{}{}{}{}{}",
        cell("a", "a"),
        group("footer-group", "F", &cell("g", "g")),
        cell("b", "b"),
        group("header-group", "H", &cell("h", "h")),
        cell("e", "e"),
    ));
    check(
        "footer then header between single cells",
        &body,
        &[
            ("t", [8.0, 8.0, 10.0, 50.0]),
            ("H", [8.0, 8.0, 10.0, 10.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [8.0, 28.0, 10.0, 10.0]),
            ("e", [8.0, 38.0, 10.0, 10.0]),
            ("F", [8.0, 48.0, 10.0, 10.0]),
            ("g", [8.0, 48.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn only_the_first_header_group_moves_to_the_top() {
    let body = table(&format!(
        "{}{}{}{}{}",
        cell("a", "a"),
        group("header-group", "H", &cell("h", "h")),
        cell("b", "b"),
        group("header-group", "H2", &cell("i", "i")),
        cell("e", "e"),
    ));
    check(
        "two header groups",
        &body,
        &[
            ("t", [8.0, 8.0, 10.0, 50.0]),
            ("H", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [8.0, 28.0, 10.0, 10.0]),
            ("H2", [8.0, 38.0, 10.0, 10.0]),
            ("i", [8.0, 38.0, 10.0, 10.0]),
            ("e", [8.0, 48.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn display_contents_wrappers_do_not_join_the_rows_around_a_group() {
    let wrapped_group = format!(
        "<div style=\"display:contents\">{}</div>",
        group("header-group", "H", &cell("h", "h"))
    );
    check(
        "wrapped header group",
        &table(&format!(
            "{}{}{}{}{}",
            cell("a", "a"),
            cell("b", "b"),
            wrapped_group,
            cell("e", "e"),
            cell("f", "f"),
        )),
        &[
            ("t", [8.0, 8.0, 20.0, 30.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [18.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
            ("f", [18.0, 28.0, 10.0, 10.0]),
        ],
    );
    check(
        "cells and the header group inside one wrapper",
        &table(&format!(
            "{}<div style=\"display:contents\">{}{}{}</div>{}",
            cell("a", "a"),
            cell("b", "b"),
            group("header-group", "H", &cell("h", "h")),
            cell("e", "e"),
            cell("f", "f"),
        )),
        &[
            ("t", [8.0, 8.0, 20.0, 30.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [18.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
            ("f", [18.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn text_on_both_sides_of_a_header_group_forms_two_rows() {
    // The two letters become anonymous cells in rows of their own, below the
    // header group: three rows of one 10px cell each.
    let body = table(&format!(
        "x{}y",
        group("header-group", "H", &cell("h", "h"))
    ));
    check(
        "text around a header group",
        &body,
        &[
            ("t", [8.0, 8.0, 10.0, 30.0]),
            ("H", [8.0, 8.0, 10.0, 10.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn column_widths_are_shared_by_the_rows_either_side_of_a_group() {
    let body = table(&format!(
        "{}{}{}",
        cell("a", "a"),
        group("header-group", "H", &cell("h", "hh")),
        cell("e", "eee"),
    ));
    check(
        "unequal cell widths",
        &body,
        &[
            ("t", [8.0, 8.0, 30.0, 30.0]),
            ("H", [8.0, 8.0, 30.0, 10.0]),
            ("h", [8.0, 8.0, 30.0, 10.0]),
            ("a", [8.0, 18.0, 30.0, 10.0]),
            ("e", [8.0, 28.0, 30.0, 10.0]),
        ],
    );
}

#[test]
fn a_body_group_or_a_row_between_bare_cells_keeps_its_place() {
    check(
        "row group",
        &table(&format!(
            "{}{}{}",
            cell("a", "a"),
            group("row-group", "B", &cell("g", "g")),
            cell("e", "e"),
        )),
        &[
            ("t", [8.0, 8.0, 10.0, 30.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("B", [8.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
    check(
        "row",
        &table(&format!(
            "{}{}{}",
            cell("a", "a"),
            group("row", "R", &cell("g", "g")),
            cell("e", "e"),
        )),
        &[
            ("t", [8.0, 8.0, 10.0, 30.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("R", [8.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
}
