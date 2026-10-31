//! A grid sized by its min-content width, with an item that states a
//! `min-width`, is no wider than that `min-width` (CSS Grid 2 §11.5, as the
//! browsers apply it): the min-content contribution of an item replaces its
//! minimum contribution only when its `min-width` is `auto`. The expected boxes
//! are the ones Chromium lays out for the same markup, with the text `AAAA BBBB`
//! in Ahem at 10px, 40px wide at its longest word and 90px wide on one line.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

const TEXT: &str = "AAAA BBBB";

/// The border box `(x, y, width, height)` of the element with the given `id`
/// when `body` is laid out on a page with an 8px margin.
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
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Check that every `(id, expected box)` of `expected` is laid out as given.
fn assert_boxes(body: &str, expected: &[(&str, [f32; 4])]) {
    let laid_out = boxes(body);
    for (id, want) in expected {
        let got = laid_out
            .iter()
            .find(|(found, _)| found == id)
            .unwrap_or_else(|| panic!("no box with id {id} in {body}"))
            .1;
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() < 0.01),
            "#{id} is {got:?}, expected {want:?} for {body}"
        );
    }
}

/// A grid of the given `grid_css` in a 400px flex container, holding one item
/// of `item_css` with the text.
fn flex_grid(grid_css: &str, item_css: &str) -> String {
    format!(
        "<div id=o style=\"display:flex;width:400px\"><div id=g style=\"display:grid;\
         width:min-content;{grid_css}\"><div id=i style=\"{item_css}\">{TEXT}</div></div></div>"
    )
}

#[test]
fn an_item_with_a_zero_minimum_width_lets_the_grid_shrink_to_nothing() {
    assert_boxes(
        &flex_grid("max-width:60px", "min-width:0"),
        &[("g", [8.0, 8.0, 0.0, 20.0]), ("i", [8.0, 8.0, 0.0, 20.0])],
    );
}

#[test]
fn an_item_with_an_automatic_minimum_width_keeps_the_longest_word() {
    assert_boxes(
        &flex_grid("max-width:60px", ""),
        &[("g", [8.0, 8.0, 40.0, 20.0]), ("i", [8.0, 8.0, 40.0, 20.0])],
    );
}

#[test]
fn the_grid_is_as_wide_as_the_minimum_width_the_item_states() {
    for (item_css, width) in [
        ("min-width:10px", 10.0),
        ("min-width:1px", 1.0),
        ("min-width:50px", 50.0),
        // The padding stays inside the box.
        ("min-width:0;padding:0 5px", 10.0),
        ("min-width:0;box-sizing:content-box;padding:0 5px", 10.0),
        // A percentage has nothing to resolve against.
        ("min-width:0%", 0.0),
        ("min-width:10%", 0.0),
        // A minimum width the keyword stands for is the content.
        ("min-width:min-content", 40.0),
    ] {
        assert_boxes(
            &flex_grid("max-width:300px", item_css),
            &[("g", [8.0, 8.0, width, 20.0])],
        );
    }
}

#[test]
fn the_maximum_width_of_an_item_does_not_raise_its_minimum() {
    assert_boxes(
        &flex_grid("max-width:300px", "min-width:0;max-width:20px"),
        &[("g", [8.0, 8.0, 0.0, 20.0])],
    );
}

#[test]
fn an_item_that_states_its_width_contributes_that_width() {
    assert_boxes(
        &flex_grid("max-width:300px", "min-width:0;width:100px"),
        &[("g", [8.0, 8.0, 100.0, 10.0])],
    );
}

#[test]
fn an_item_that_is_a_scroll_container_has_no_content_minimum() {
    for overflow in ["overflow:hidden", "min-width:0;overflow:visible"] {
        assert_boxes(
            &flex_grid("max-width:300px", overflow),
            &[("g", [8.0, 8.0, 0.0, 20.0])],
        );
    }
}

#[test]
fn the_grid_does_not_depend_on_the_parent_that_measures_it() {
    for parent in [
        "display:grid;width:400px",
        "display:inline-flex;width:400px",
        "display:flex;flex-direction:column;width:400px",
    ] {
        assert_boxes(
            &format!(
                "<div id=o style=\"{parent}\"><div id=g style=\"display:grid;width:min-content\">\
                 <div id=i style=\"min-width:0\">{TEXT}</div></div></div>"
            ),
            &[("g", [8.0, 8.0, 0.0, 20.0]), ("i", [8.0, 8.0, 0.0, 20.0])],
        );
    }
    assert_boxes(
        &format!(
            "<div id=g style=\"display:grid;width:min-content\">\
             <div id=i style=\"min-width:0\">{TEXT}</div></div>"
        ),
        &[("g", [8.0, 8.0, 0.0, 20.0])],
    );
    assert_boxes(
        &format!(
            "<div id=o style=\"display:flex;width:200px\"><div id=g \
             style=\"display:inline-grid;width:min-content\">\
             <div id=i style=\"min-width:0\">{TEXT}</div></div></div>"
        ),
        &[("g", [8.0, 8.0, 0.0, 20.0])],
    );
}

#[test]
fn the_items_of_two_columns_are_held_to_their_own_minimums() {
    let columns = "grid-template-columns:repeat(2,auto);max-width:100px";
    let two = |second: &str| {
        format!(
            "<div id=o style=\"display:flex;width:200px\"><div id=g style=\"display:grid;\
             width:min-content;{columns}\"><div id=i style=\"min-width:0\">{TEXT}</div>\
             <div id=j style=\"{second}\">{TEXT}</div></div></div>"
        )
    };
    assert_boxes(
        &two("min-width:0"),
        &[
            ("g", [8.0, 8.0, 0.0, 20.0]),
            ("i", [8.0, 8.0, 0.0, 20.0]),
            ("j", [8.0, 8.0, 0.0, 20.0]),
        ],
    );
    assert_boxes(
        &two(""),
        &[
            ("g", [8.0, 8.0, 40.0, 20.0]),
            ("i", [8.0, 8.0, 0.0, 20.0]),
            ("j", [8.0, 8.0, 40.0, 20.0]),
        ],
    );
}

#[test]
fn a_track_that_is_sized_by_content_ignores_the_minimum_width_of_its_item() {
    for (columns, width, height) in [
        ("min-content", 40.0, 20.0),
        ("max-content", 90.0, 10.0),
        ("minmax(min-content,auto)", 40.0, 20.0),
        // Tracks that have an automatic minimum follow the item's minimum.
        ("auto", 0.0, 20.0),
        ("minmax(auto,10px)", 0.0, 20.0),
        ("fit-content(20px)", 0.0, 20.0),
        ("1fr", 0.0, 20.0),
        ("minmax(0,auto)", 0.0, 20.0),
    ] {
        assert_boxes(
            &flex_grid(
                &format!("max-width:300px;grid-template-columns:{columns}"),
                "min-width:0",
            ),
            &[("g", [8.0, 8.0, width, height])],
        );
    }
}

#[test]
fn an_item_that_spans_two_columns_is_held_to_its_minimum_too() {
    assert_boxes(
        &flex_grid(
            "grid-template-columns:auto auto",
            "min-width:0;grid-column:span 2",
        ),
        &[("g", [8.0, 8.0, 0.0, 20.0])],
    );
}

#[test]
fn a_replaced_item_with_a_zero_minimum_width_overflows_a_grid_of_no_width() {
    assert_boxes(
        "<div id=o style=\"display:flex;width:400px\"><div id=g style=\"display:grid;\
         width:min-content\"><div id=i style=\"min-width:0\"><img id=im \
         src=\"data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7\" \
         style=\"width:100px;height:10px\"></div></div></div>",
        &[
            ("g", [8.0, 8.0, 0.0, 12.0]),
            ("i", [8.0, 8.0, 0.0, 12.0]),
            ("im", [8.0, 8.0, 100.0, 10.0]),
        ],
    );
}

#[test]
fn a_grid_that_shrinks_in_a_flex_container_follows_the_minimum_of_its_items() {
    let narrow = |item_css: &str| {
        format!(
            "<div id=o style=\"display:flex;width:30px\"><div id=g style=\"display:grid\">\
             <div id=i style=\"{item_css}\">{TEXT}</div></div></div>"
        )
    };
    assert_boxes(
        &narrow("min-width:0"),
        &[("g", [8.0, 8.0, 30.0, 20.0]), ("i", [8.0, 8.0, 30.0, 20.0])],
    );
    assert_boxes(
        &narrow(""),
        &[("g", [8.0, 8.0, 40.0, 20.0]), ("i", [8.0, 8.0, 40.0, 20.0])],
    );
}

#[test]
fn the_rows_are_measured_at_the_width_the_grid_gets() {
    assert_boxes(
        &format!(
            "<div id=o style=\"display:flex;width:400px\"><div id=g style=\"display:grid;\
             width:min-content\"><div id=i style=\"min-width:0\">{TEXT}</div>\
             <div id=j style=\"min-width:0;width:50px\">{TEXT}</div></div></div>"
        ),
        &[
            ("g", [8.0, 8.0, 50.0, 40.0]),
            ("i", [8.0, 8.0, 50.0, 20.0]),
            ("j", [8.0, 28.0, 50.0, 20.0]),
        ],
    );
}

#[test]
fn the_width_held_to_the_minimums_is_still_bounded_by_the_grid_itself() {
    for (grid_css, width, height) in [
        ("min-width:30px", 30.0, 20.0),
        ("min-width:60px", 60.0, 20.0),
        ("max-width:10px", 0.0, 20.0),
        // The border and the padding of the grid are on top of its content.
        ("padding:0 7px;border:1px solid", 16.0, 22.0),
        (
            "box-sizing:content-box;padding:0 7px;border:1px solid",
            16.0,
            22.0,
        ),
    ] {
        assert_boxes(
            &flex_grid(grid_css, "min-width:0"),
            &[("g", [8.0, 8.0, width, height])],
        );
    }
}

#[test]
fn the_margin_of_a_held_item_is_part_of_the_width_of_the_grid() {
    assert_boxes(
        &flex_grid("", "min-width:0;margin:0 6px"),
        &[("g", [8.0, 8.0, 12.0, 20.0]), ("i", [14.0, 8.0, 0.0, 20.0])],
    );
}

#[test]
fn a_percentage_height_of_a_held_item_resolves_against_the_grid_height() {
    assert_boxes(
        &flex_grid("height:60px", "min-width:0;height:50%"),
        &[("g", [8.0, 8.0, 0.0, 60.0]), ("i", [8.0, 8.0, 0.0, 30.0])],
    );
}

#[test]
fn a_chain_of_nested_grids_is_laid_out_in_time_linear_in_its_length() {
    // Each grid is measured by its min-content width, and holds the grid inside
    // it to a `min-width` while it is. A measure that began every grid's work
    // over for each width its parent tried would take time exponential in the
    // length of the chain, which at this length is never.
    let depth = 24;
    let nested = "<div style=\"display:grid;width:min-content;min-width:0\">".repeat(depth);
    let closing = "</div>".repeat(depth);
    assert_boxes(
        &format!(
            "<div id=o style=\"display:flex;width:400px\"><div id=g style=\"display:grid;\
             width:min-content\"><div id=i style=\"min-width:0\">{nested}\
             <div style=\"min-width:0\">{TEXT}</div>{closing}</div></div></div>"
        ),
        &[("g", [8.0, 8.0, 0.0, 20.0]), ("i", [8.0, 8.0, 0.0, 20.0])],
    );
}
