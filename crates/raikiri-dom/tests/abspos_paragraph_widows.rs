//! A paragraph in an absolutely positioned box breaks between its lines at a
//! page end, and Chromium keeps `widows` there as in the flow: the break
//! moves up to leave the last lines of the paragraph on the next page, as far
//! as `orphans` lets the page before keep its lines. `orphans` never moves a
//! break down, as the box cannot start its lines on a later page. A line that
//! starts a page exactly breaks the paragraph there as well, and the lines a
//! clip hides still count among the widows.
//! The pages and positions are the ones Chromium prints for the same markup:
//! 100px pages without margins, Ahem at 20px on 30px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

const LETTERS: [&str; 8] = ["a", "b", "c", "d", "e", "f", "g", "h"];

/// The page index and the top on that page of the first fragment of each of
/// the line elements `#a`, `#b`, and so on, `count` of them.
fn places(body: &str, count: usize) -> Vec<(usize, f32)> {
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style>{body}"
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
    let mut found = vec![None; count];
    for (index, page) in result.pages().enumerate() {
        for fragment in page.fragments() {
            let Some(slot) = page
                .dom()
                .attr(fragment.node(), "id")
                .and_then(|id| LETTERS[..count].iter().position(|letter| *letter == id))
            else {
                continue;
            };
            found[slot].get_or_insert((index, fragment.rect().y));
        }
    }
    found
        .into_iter()
        .map(|place| place.unwrap_or_else(|| panic!("a line is missing in {body}")))
        .collect()
}

/// `count` lines, each one letter in a span with the letter as its id.
fn lines(count: usize) -> String {
    LETTERS[..count]
        .iter()
        .map(|letter| format!("<span id={letter}>{}</span>", letter.to_uppercase()))
        .collect::<Vec<_>>()
        .join("<br>")
}

/// A box positioned against the first page, below a flow two pages tall.
fn on_first_page(top: u32, count: usize, style: &str) -> String {
    format!(
        "<div style=\"height:200px\"></div><div style=\"position:absolute;top:{top}px;\
         left:0;width:100px;{style}\">{}</div>",
        lines(count)
    )
}

/// A box positioned against a relatively positioned block that starts the
/// second page.
fn on_second_page(top: u32, count: usize, style: &str) -> String {
    format!(
        "<div style=\"height:100px\"></div><div style=\"position:relative\">\
         <div style=\"position:absolute;top:{top}px;width:100px;{style}\">{}</div></div>\
         <div style=\"height:200px\"></div>",
        lines(count)
    )
}

/// Lays out every `(body, places)` case and reports all that differ.
fn assert_places(cases: &[(String, Vec<(usize, f32)>)]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(body, expected)| {
            let got = places(body, expected.len());
            got.iter()
                .zip(expected)
                .any(|(got, expected)| got.0 != expected.0 || (got.1 - expected.1).abs() >= 0.01)
                .then(|| format!("{body}: expected {expected:?}, got {got:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_break_moves_up_to_keep_the_widows() {
    assert_places(&[
        (
            on_first_page(0, 4, "orphans:2;widows:3"),
            vec![(0, 5.0), (0, 35.0), (1, 5.0), (1, 35.0)],
        ),
        (
            on_first_page(40, 3, "orphans:1;widows:3"),
            vec![(0, 45.0), (1, 5.0), (1, 35.0)],
        ),
        (
            on_first_page(50, 5, "orphans:1;widows:3"),
            vec![(0, 55.0), (1, 5.0), (2, 5.0), (2, 35.0), (2, 65.0)],
        ),
        (
            on_second_page(0, 4, "widows:2"),
            vec![(1, 5.0), (1, 35.0), (2, 5.0), (2, 35.0)],
        ),
    ]);
}

#[test]
fn the_orphans_hold_the_widows_back() {
    assert_places(&[
        (
            on_first_page(50, 5, "orphans:2;widows:3"),
            vec![(0, 55.0), (1, 5.0), (1, 35.0), (2, 5.0), (2, 35.0)],
        ),
        (
            on_first_page(50, 5, "orphans:3;widows:3"),
            vec![(0, 55.0), (1, 5.0), (1, 35.0), (1, 65.0), (2, 5.0)],
        ),
        (
            on_second_page(80, 4, "widows:3"),
            vec![(2, 5.0), (2, 35.0), (3, 5.0), (3, 35.0)],
        ),
    ]);
}

#[test]
fn the_orphans_never_move_the_break_down() {
    assert_places(&[(
        on_first_page(40, 4, "orphans:3;widows:3"),
        vec![(0, 45.0), (0, 75.0), (1, 5.0), (1, 35.0)],
    )]);
}

#[test]
fn a_line_that_starts_a_page_keeps_the_widows_too() {
    assert_places(&[
        (
            on_first_page(10, 4, "orphans:1;widows:2"),
            vec![(0, 15.0), (0, 45.0), (1, 5.0), (1, 35.0)],
        ),
        (
            on_first_page(10, 4, "orphans:1;widows:3"),
            vec![(0, 15.0), (1, 5.0), (1, 35.0), (1, 65.0)],
        ),
    ]);
}

#[test]
fn the_widows_count_from_the_page_the_first_line_moves_to() {
    assert_places(&[
        (
            on_first_page(80, 4, "orphans:1;widows:3"),
            vec![(1, 5.0), (2, 5.0), (2, 35.0), (2, 65.0)],
        ),
        (
            on_first_page(70, 4, "orphans:1;widows:3;padding-top:15px"),
            vec![(1, 5.0), (2, 5.0), (2, 35.0), (2, 65.0)],
        ),
    ]);
}

#[test]
fn the_lines_a_clip_hides_count_as_widows() {
    assert_places(&[
        (
            on_first_page(40, 8, "height:180px;overflow:hidden;orphans:1;widows:3"),
            vec![
                (0, 45.0),
                (0, 75.0),
                (1, 5.0),
                (1, 35.0),
                (1, 65.0),
                (2, 5.0),
            ],
        ),
        (
            on_first_page(60, 8, "height:210px;overflow:clip;orphans:1;widows:3"),
            vec![
                (0, 65.0),
                (1, 5.0),
                (1, 35.0),
                (1, 65.0),
                (2, 5.0),
                (2, 35.0),
                (2, 65.0),
            ],
        ),
    ]);
}
